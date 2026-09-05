use std::ffi::CString;
use std::os::raw::c_void;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::sync::Arc;

use calyx_core::Result;
use cudarc::driver::{CudaStream, sys::CUdeviceptr};
use cuvs_sys as ffi;

use super::{check, invalid, unavailable};

pub(super) struct Resources(pub(super) ffi::cuvsResources_t);

impl Resources {
    pub(super) fn new(stream: &Arc<CudaStream>) -> Result<Self> {
        let mut resources = 0;
        check(
            unsafe { ffi::cuvsResourcesCreate(&mut resources) },
            "create resources",
        )?;
        check(
            unsafe { ffi::cuvsStreamSet(resources, stream.cu_stream() as _) },
            "set serving stream",
        )?;
        Ok(Self(resources))
    }
}

impl Drop for Resources {
    fn drop(&mut self) {
        let _ = unsafe { ffi::cuvsResourcesDestroy(self.0) };
    }
}

pub(super) struct CagraIndex(pub(super) ffi::cuvsCagraIndex_t);

impl CagraIndex {
    pub(super) fn deserialize(resources: &Resources, path: &Path) -> Result<Self> {
        let mut index = ptr::null_mut();
        check(
            unsafe { ffi::cuvsCagraIndexCreate(&mut index) },
            "create index",
        )?;
        if index.is_null() {
            return Err(unavailable("cuVS returned a null CAGRA index"));
        }
        let index = Self(index);
        let filename = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| unavailable(format!("CAGRA path contains NUL: {}", path.display())))?;
        check(
            unsafe { ffi::cuvsCagraDeserialize(resources.0, filename.as_ptr(), index.0) },
            "deserialize asset",
        )?;
        check(
            unsafe { ffi::cuvsStreamSync(resources.0) },
            "deserialize sync",
        )?;
        Ok(index)
    }

    pub(super) fn metadata(&self) -> Result<(usize, usize, usize)> {
        let (mut rows, mut dim, mut degree) = (0_i64, 0_i64, 0_i64);
        check(
            unsafe { ffi::cuvsCagraIndexGetSize(self.0, &mut rows) },
            "read rows",
        )?;
        check(
            unsafe { ffi::cuvsCagraIndexGetDims(self.0, &mut dim) },
            "read dim",
        )?;
        check(
            unsafe { ffi::cuvsCagraIndexGetGraphDegree(self.0, &mut degree) },
            "read graph degree",
        )?;
        Ok((
            checked_usize(rows, "rows")?,
            checked_usize(dim, "dim")?,
            checked_usize(degree, "degree")?,
        ))
    }

    pub(super) fn dataset_device_layout(
        &self,
        rows: usize,
        dim: usize,
    ) -> Result<(CUdeviceptr, i64, i64)> {
        let mut dataset = empty_tensor();
        check(
            unsafe { ffi::cuvsCagraIndexGetDataset(self.0, &mut dataset) },
            "read dataset view",
        )?;
        let result = (|| -> Result<(CUdeviceptr, i64, i64)> {
            let (row_stride, column_stride) = validate_dataset_view(&dataset, rows, dim)?;
            let base = dataset.dl_tensor.data as usize;
            let offset = usize::try_from(dataset.dl_tensor.byte_offset)
                .map_err(|_| unavailable("CAGRA dataset byte offset exceeds usize"))?;
            let address = base
                .checked_add(offset)
                .ok_or_else(|| unavailable("CAGRA dataset device address overflow"))?;
            Ok((address as CUdeviceptr, row_stride, column_stride))
        })();
        drop_view(&mut dataset);
        result
    }
}

impl Drop for CagraIndex {
    fn drop(&mut self) {
        let _ = unsafe { ffi::cuvsCagraIndexDestroy(self.0) };
    }
}

pub(super) struct SearchParams(pub(super) ffi::cuvsCagraSearchParams_t);

impl SearchParams {
    pub(super) fn new() -> Result<Self> {
        let mut params = ptr::null_mut();
        check(
            unsafe { ffi::cuvsCagraSearchParamsCreate(&mut params) },
            "create search params",
        )?;
        if params.is_null() {
            return Err(unavailable("cuVS returned null CAGRA search params"));
        }
        Ok(Self(params))
    }

    pub(super) fn configure(
        &mut self,
        query_count: usize,
        k: usize,
        ef_search: usize,
    ) -> Result<()> {
        let requested_itopk = ef_search.max(k);
        let itopk_size = requested_itopk
            .checked_next_power_of_two()
            .ok_or_else(|| invalid(format!("CAGRA itopk size overflow for {requested_itopk}")))?
            .max(32);
        unsafe {
            (*self.0).max_queries = query_count;
            (*self.0).itopk_size = itopk_size;
            // cuVS 26.06's C constructor leaves both enum fields at zero,
            // selecting SINGLE_CTA/HASH rather than the documented C++
            // AUTO defaults. Establish the contract explicitly so cuVS can
            // select MULTI_CTA when itopk exceeds SINGLE_CTA's hard 512 cap.
            (*self.0).algo = ffi::cuvsCagraSearchAlgo::AUTO;
            (*self.0).hashmap_mode = ffi::cuvsCagraHashMode::AUTO_HASH;
        }
        Ok(())
    }
}

impl Drop for SearchParams {
    fn drop(&mut self) {
        let _ = unsafe { ffi::cuvsCagraSearchParamsDestroy(self.0) };
    }
}

fn validate_dataset_view(
    view: &ffi::DLManagedTensor,
    rows: usize,
    dim: usize,
) -> Result<(i64, i64)> {
    let tensor = view.dl_tensor;
    if tensor.ndim != 2 || tensor.shape.is_null() || tensor.data.is_null() {
        return Err(unavailable("CAGRA dataset view is null or not rank two"));
    }
    let shape = unsafe { std::slice::from_raw_parts(tensor.shape, 2) };
    let strides = if tensor.strides.is_null() {
        [dim as i64, 1]
    } else {
        unsafe {
            let strides = std::slice::from_raw_parts(tensor.strides, 2);
            [strides[0], strides[1]]
        }
    };
    if shape != [rows as i64, dim as i64]
        || strides[0] <= 0
        || strides[1] <= 0
        || tensor.dtype.code != ffi::DLDataTypeCode::kDLFloat as u8
        || tensor.dtype.bits != 32
        || tensor.device.device_type != ffi::DLDeviceType::kDLCUDA
    {
        return Err(unavailable(format!(
            "unsupported CAGRA dataset view shape={shape:?} strides={strides:?} dtype={:?}/{} device={:?}",
            tensor.dtype.code, tensor.dtype.bits, tensor.device.device_type
        )));
    }
    Ok((strides[0], strides[1]))
}

pub(super) fn device_tensor(
    data: CUdeviceptr,
    shape: &mut [i64; 2],
    dtype: ffi::DLDataType,
) -> ffi::DLManagedTensor {
    ffi::DLManagedTensor {
        dl_tensor: ffi::DLTensor {
            data: data as usize as *mut c_void,
            device: ffi::DLDevice {
                device_type: ffi::DLDeviceType::kDLCUDA,
                device_id: 0,
            },
            ndim: 2,
            dtype,
            shape: shape.as_mut_ptr(),
            strides: ptr::null_mut(),
            byte_offset: 0,
        },
        manager_ctx: ptr::null_mut(),
        deleter: None,
    }
}

fn empty_tensor() -> ffi::DLManagedTensor {
    ffi::DLManagedTensor {
        dl_tensor: ffi::DLTensor {
            data: ptr::null_mut(),
            device: ffi::DLDevice {
                device_type: ffi::DLDeviceType::kDLCUDA,
                device_id: 0,
            },
            ndim: 0,
            dtype: dtype_f32(),
            shape: ptr::null_mut(),
            strides: ptr::null_mut(),
            byte_offset: 0,
        },
        manager_ctx: ptr::null_mut(),
        deleter: None,
    }
}

pub(super) fn dtype_f32() -> ffi::DLDataType {
    ffi::DLDataType {
        code: ffi::DLDataTypeCode::kDLFloat as u8,
        bits: 32,
        lanes: 1,
    }
}

pub(super) fn dtype_i64() -> ffi::DLDataType {
    ffi::DLDataType {
        code: ffi::DLDataTypeCode::kDLInt as u8,
        bits: 64,
        lanes: 1,
    }
}

fn drop_view(view: &mut ffi::DLManagedTensor) {
    if let Some(deleter) = view.deleter.take() {
        unsafe { deleter(view) };
    }
}

fn checked_usize(value: i64, name: &'static str) -> Result<usize> {
    usize::try_from(value).map_err(|_| unavailable(format!("CAGRA {name} is negative")))
}
