//! cuVS brute-force exact KNN helper for accepted-reference FSV artifacts.

use calyx_core::Result;

#[derive(Clone, Debug)]
pub struct CuvsBruteForceTopK {
    pub query_count: usize,
    pub k: usize,
    pub neighbors: Vec<i64>,
    pub distances: Vec<f32>,
}

impl CuvsBruteForceTopK {
    pub fn row(&self, query_idx: usize) -> (&[i64], &[f32]) {
        let start = query_idx * self.k;
        let end = start + self.k;
        (&self.neighbors[start..end], &self.distances[start..end])
    }
}

#[cfg(feature = "cuda")]
pub fn cuvs_bruteforce_topk(
    dataset: &mut [f32],
    rows: usize,
    dim: usize,
    queries: &mut [f32],
    query_count: usize,
    k: usize,
) -> Result<CuvsBruteForceTopK> {
    imp::run(dataset, rows, dim, queries, query_count, k)
}

#[cfg(not(feature = "cuda"))]
pub fn cuvs_bruteforce_topk(
    _dataset: &mut [f32],
    _rows: usize,
    _dim: usize,
    _queries: &mut [f32],
    _query_count: usize,
    _k: usize,
) -> Result<CuvsBruteForceTopK> {
    Err(crate::error::sextant_error(
        crate::error::CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE,
        "cuvs brute-force reference generation requires building calyx-cli with --features cuda",
    ))
}

#[cfg(feature = "cuda")]
mod imp {
    use std::ffi::CStr;
    use std::os::raw::c_void;
    use std::ptr;
    use std::sync::Arc;

    use calyx_core::Result;
    use cudarc::driver::{
        CudaContext, CudaSlice, CudaStream, DevicePtr, DevicePtrMut, ValidAsZeroBits,
        sys::CUdeviceptr,
    };
    use cuvs_sys as ffi;

    use super::CuvsBruteForceTopK;
    use crate::error::{CALYX_INDEX_INVALID_PARAMS, CALYX_INDEX_IO, sextant_error};

    pub(super) fn run(
        dataset: &mut [f32],
        rows: usize,
        dim: usize,
        queries: &mut [f32],
        query_count: usize,
        k: usize,
    ) -> Result<CuvsBruteForceTopK> {
        validate_shapes(dataset, rows, dim, queries, query_count, k)?;
        let cuda = cuda_context()?;
        let stream = cuda.default_stream();
        let dataset_dev = copy_to_device(&stream, dataset, "dataset")?;
        sync_stream(&stream, "sync after dataset copy")?;
        let res = Resources::new()?;
        let index = BruteForceIndex::new()?;
        {
            let mut dataset_shape = [rows as i64, dim as i64];
            let (dataset_ptr, _dataset_guard) = dataset_dev.device_ptr(&stream);
            let mut dataset_tensor = device_tensor(dataset_ptr, &mut dataset_shape, dtype_f32());
            check(
                unsafe {
                    ffi::cuvsBruteForceBuild(
                        res.0,
                        &mut dataset_tensor,
                        ffi::cuvsDistanceType::L2Expanded,
                        0.0,
                        index.0,
                    )
                },
                "build",
            )?;
            check(unsafe { ffi::cuvsStreamSync(res.0) }, "sync after build")?;
        }

        let query_dev = copy_to_device(&stream, queries, "queries")?;
        sync_stream(&stream, "sync after query copy")?;
        let mut neighbors_dev = alloc_device::<i64>(&stream, query_count * k, "neighbors")?;
        let mut distances_dev = alloc_device::<f32>(&stream, query_count * k, "distances")?;
        {
            let mut query_shape = [query_count as i64, dim as i64];
            let mut neighbor_shape = [query_count as i64, k as i64];
            let mut distance_shape = [query_count as i64, k as i64];
            let (query_ptr, _query_guard) = query_dev.device_ptr(&stream);
            let (neighbor_ptr, _neighbor_guard) = neighbors_dev.device_ptr_mut(&stream);
            let (distance_ptr, _distance_guard) = distances_dev.device_ptr_mut(&stream);
            let mut query_tensor = device_tensor(query_ptr, &mut query_shape, dtype_f32());
            let mut neighbor_tensor = device_tensor(neighbor_ptr, &mut neighbor_shape, dtype_i64());
            let mut distance_tensor = device_tensor(distance_ptr, &mut distance_shape, dtype_f32());
            let filter = ffi::cuvsFilter {
                addr: 0,
                type_: ffi::cuvsFilterType::NO_FILTER,
            };
            check(
                unsafe {
                    ffi::cuvsBruteForceSearch(
                        res.0,
                        index.0,
                        &mut query_tensor,
                        &mut neighbor_tensor,
                        &mut distance_tensor,
                        filter,
                    )
                },
                "search",
            )?;
            check(unsafe { ffi::cuvsStreamSync(res.0) }, "sync after search")?;
        }
        let neighbors = copy_to_host(&stream, &neighbors_dev, "neighbors")?;
        let distances = copy_to_host(&stream, &distances_dev, "distances")?;
        validate_output(&neighbors, &distances, rows)?;
        Ok(CuvsBruteForceTopK {
            query_count,
            k,
            neighbors,
            distances,
        })
    }

    fn validate_shapes(
        dataset: &[f32],
        rows: usize,
        dim: usize,
        queries: &[f32],
        query_count: usize,
        k: usize,
    ) -> Result<()> {
        if rows == 0 || dim == 0 || query_count == 0 || k == 0 || k > rows {
            return Err(sextant_error(
                CALYX_INDEX_INVALID_PARAMS,
                format!(
                    "invalid cuvs brute-force shape rows={rows} dim={dim} queries={query_count} k={k}"
                ),
            ));
        }
        if dataset.len() != rows * dim || queries.len() != query_count * dim {
            return Err(sextant_error(
                CALYX_INDEX_INVALID_PARAMS,
                "cuvs brute-force input buffers do not match rows*dim",
            ));
        }
        if dataset
            .iter()
            .chain(queries)
            .any(|value| !value.is_finite())
        {
            return Err(sextant_error(
                CALYX_INDEX_INVALID_PARAMS,
                "cuvs brute-force inputs contain non-finite values",
            ));
        }
        Ok(())
    }

    fn validate_output(neighbors: &[i64], distances: &[f32], rows: usize) -> Result<()> {
        for (idx, (&neighbor, &distance)) in neighbors.iter().zip(distances).enumerate() {
            let neighbor_ok = usize::try_from(neighbor).is_ok_and(|value| value < rows);
            if !neighbor_ok || !distance.is_finite() {
                return Err(sextant_error(
                    CALYX_INDEX_IO,
                    format!(
                        "cuvs brute-force output idx={idx} neighbor={neighbor} distance={distance}"
                    ),
                ));
            }
        }
        Ok(())
    }

    struct Resources(ffi::cuvsResources_t);

    impl Resources {
        fn new() -> Result<Self> {
            let mut res = 0;
            check(
                unsafe { ffi::cuvsResourcesCreate(&mut res) },
                "create resources",
            )?;
            Ok(Self(res))
        }
    }

    fn cuda_context() -> Result<Arc<CudaContext>> {
        CudaContext::new(0).map_err(|err| {
            sextant_error(
                CALYX_INDEX_IO,
                format!("cuvs brute-force CUDA context init failed: {err}"),
            )
        })
    }

    fn copy_to_device<T: cudarc::driver::DeviceRepr>(
        stream: &Arc<CudaStream>,
        data: &[T],
        name: &'static str,
    ) -> Result<CudaSlice<T>> {
        stream.clone_htod(data).map_err(|err| {
            sextant_error(
                CALYX_INDEX_IO,
                format!("cuvs brute-force copy {name} to CUDA failed: {err}"),
            )
        })
    }

    fn alloc_device<T: cudarc::driver::DeviceRepr + ValidAsZeroBits>(
        stream: &Arc<CudaStream>,
        len: usize,
        name: &'static str,
    ) -> Result<CudaSlice<T>> {
        stream.alloc_zeros(len).map_err(|err| {
            sextant_error(
                CALYX_INDEX_IO,
                format!("cuvs brute-force allocate CUDA {name} len={len} failed: {err}"),
            )
        })
    }

    fn sync_stream(stream: &Arc<CudaStream>, stage: &'static str) -> Result<()> {
        stream.synchronize().map_err(|err| {
            sextant_error(
                CALYX_INDEX_IO,
                format!("cuvs brute-force CUDA {stage} failed: {err}"),
            )
        })
    }

    fn copy_to_host<T: cudarc::driver::DeviceRepr>(
        stream: &Arc<CudaStream>,
        data: &CudaSlice<T>,
        name: &'static str,
    ) -> Result<Vec<T>> {
        stream.clone_dtoh(data).map_err(|err| {
            sextant_error(
                CALYX_INDEX_IO,
                format!("cuvs brute-force copy CUDA {name} to host failed: {err}"),
            )
        })
    }

    impl Drop for Resources {
        fn drop(&mut self) {
            let _ = unsafe { ffi::cuvsResourcesDestroy(self.0) };
        }
    }

    struct BruteForceIndex(ffi::cuvsBruteForceIndex_t);

    impl BruteForceIndex {
        fn new() -> Result<Self> {
            let mut index = ptr::null_mut();
            check(
                unsafe { ffi::cuvsBruteForceIndexCreate(&mut index) },
                "create index",
            )?;
            if index.is_null() {
                return Err(cuvs_error("create index", "returned null index"));
            }
            Ok(Self(index))
        }
    }

    impl Drop for BruteForceIndex {
        fn drop(&mut self) {
            let _ = unsafe { ffi::cuvsBruteForceIndexDestroy(self.0) };
        }
    }

    fn device_tensor(
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

    fn dtype_f32() -> ffi::DLDataType {
        ffi::DLDataType {
            code: ffi::DLDataTypeCode::kDLFloat as u8,
            bits: 32,
            lanes: 1,
        }
    }

    fn dtype_i64() -> ffi::DLDataType {
        ffi::DLDataType {
            code: ffi::DLDataTypeCode::kDLInt as u8,
            bits: 64,
            lanes: 1,
        }
    }

    fn check(status: ffi::cuvsError_t, stage: &'static str) -> Result<()> {
        if status == ffi::cuvsError_t::CUVS_SUCCESS {
            Ok(())
        } else {
            Err(cuvs_error(stage, format!("status {status:?}")))
        }
    }

    fn cuvs_error(stage: &str, detail: impl std::fmt::Display) -> calyx_core::CalyxError {
        let last = unsafe {
            let ptr = ffi::cuvsGetLastErrorText();
            if ptr.is_null() {
                "no cuVS error text".to_string()
            } else {
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        };
        sextant_error(
            CALYX_INDEX_IO,
            format!("cuvs brute-force {stage}: {detail}; last_error={last}"),
        )
    }
}
