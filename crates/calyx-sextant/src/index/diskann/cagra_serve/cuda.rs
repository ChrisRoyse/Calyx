use std::ffi::CStr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use calyx_core::Result;
use cudarc::driver::{
    CudaContext, CudaFunction, CudaSlice, CudaStream, DevicePtr, DevicePtrMut, LaunchConfig,
    PushKernelArg, sys::CUdeviceptr,
};
use cudarc::nvrtc::Ptx;
use cuvs_sys as ffi;

use super::CagraServingMetric;
use super::output::{validate_output, validated_bitset};
use super::telemetry::TELEMETRY;
use crate::error::{
    CALYX_INDEX_DIM_MISMATCH, CALYX_INDEX_INVALID_PARAMS, CALYX_SEXTANT_GPU_SERVING_UNAVAILABLE,
    sextant_error,
};
use crate::index::distance::l2_normalize;

#[path = "ffi.rs"]
mod ffi_support;
use ffi_support::{CagraIndex, Resources, SearchParams, device_tensor, dtype_f32, dtype_i64};

const EXACT_CUBIN: &[u8] = include_bytes!(env!("SEXTANT_CHUNKED_EXACT_MERGE_CUBIN_PATH"));

pub(super) struct Asset {
    // Declaration order is drop order: destroy the index while its CUDA
    // resources/stream/context still exist. `dataset_ptr` is a non-owning view
    // into the index and is never freed independently.
    index: CagraIndex,
    search_params: SearchParams,
    exact: CudaFunction,
    resources: Resources,
    stream: Arc<CudaStream>,
    _context: Arc<CudaContext>,
    rows: usize,
    dim: usize,
    dataset_ptr: CUdeviceptr,
    dataset_row_stride: i64,
    dataset_column_stride: i64,
    base_bytes: u64,
    query: Option<CudaSlice<f32>>,
    ids: Option<CudaSlice<i64>>,
    distances: Option<CudaSlice<f32>>,
    filter: Option<CudaSlice<u32>>,
}

// All FFI and CUDA state is exclusively accessed through the cache's
// `Mutex<Asset>`. No raw handle is ever used concurrently or moved mid-call.
unsafe impl Send for Asset {}

impl Asset {
    pub(super) fn load(path: &Path) -> Result<Self> {
        let context = CudaContext::new(0).map_err(cuda_error("context init"))?;
        let stream = context.new_stream().map_err(cuda_error("stream init"))?;
        let resources = Resources::new(&stream)?;
        let index = CagraIndex::deserialize(&resources, path)?;
        let (rows, dim, graph_degree) = index.metadata()?;
        if rows == 0 || dim == 0 || graph_degree == 0 {
            return Err(unavailable(format!(
                "CAGRA asset {} has invalid metadata rows={rows} dim={dim} degree={graph_degree}",
                path.display()
            )));
        }
        let (dataset_ptr, dataset_row_stride, dataset_column_stride) =
            index.dataset_device_layout(rows, dim)?;
        let search_params = SearchParams::new()?;
        let module = context
            .load_module(Ptx::from_binary(EXACT_CUBIN.to_vec()))
            .map_err(cuda_error("filtered exact CUBIN load"))?;
        let exact = module
            .load_function("exact_cosine_resident_filtered")
            .map_err(cuda_error("filtered exact kernel load"))?;
        let estimated_base = rows
            .saturating_mul(dim.saturating_add(graph_degree))
            .saturating_mul(size_of::<f32>());
        // The serialized sidecar is the cache's conservative floor for the
        // opaque cuVS allocation. Retained serving buffers are accounted on
        // top of that floor instead of being hidden beneath it.
        let serialized_base = path
            .metadata()
            .map_err(|error| unavailable(format!("stat CAGRA asset {}: {error}", path.display())))?
            .len();
        let base_bytes = serialized_base.max(u64::try_from(estimated_base).unwrap_or(u64::MAX));
        Ok(Self {
            index,
            search_params,
            exact,
            resources,
            stream,
            _context: context,
            rows,
            dim,
            dataset_ptr,
            dataset_row_stride,
            dataset_column_stride,
            base_bytes,
            query: None,
            ids: None,
            distances: None,
            filter: None,
        })
    }

    pub(super) fn resident_bytes(&self) -> u64 {
        self.projected_bytes(0, 0, 0)
    }

    pub(super) fn projected_search_bytes(
        &self,
        query_values: usize,
        pairs: usize,
        filtered: bool,
    ) -> u64 {
        self.projected_bytes(
            query_values,
            pairs,
            if filtered { self.rows.div_ceil(32) } else { 0 },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn search(
        &mut self,
        metric: CagraServingMetric,
        queries: &[f32],
        query_count: usize,
        k: usize,
        ef_search: usize,
        allowed_ids: Option<&[u32]>,
    ) -> Result<Vec<Vec<(u32, f32)>>> {
        let query_dim = queries.len() / query_count;
        if query_dim != self.dim {
            return Err(sextant_error(
                CALYX_INDEX_DIM_MISMATCH,
                format!("CAGRA query dim {query_dim} != asset dim {}", self.dim),
            ));
        }
        let allowed = allowed_ids
            .map(|ids| validated_bitset(ids, self.rows))
            .transpose()?;
        let available = allowed.as_ref().map_or(self.rows, |(_, count)| *count);
        let output_k = k.min(available);
        TELEMETRY.batches.fetch_add(1, Ordering::Relaxed);
        TELEMETRY
            .queries
            .fetch_add(query_count as u64, Ordering::Relaxed);
        if output_k == 0 {
            return Ok(vec![Vec::new(); query_count]);
        }

        let query_values = match metric {
            CagraServingMetric::UnitL2 => queries
                .chunks_exact(self.dim)
                .flat_map(l2_normalize)
                .collect::<Vec<_>>(),
            CagraServingMetric::RawL2 => queries.to_vec(),
        };
        let pair_count = query_count
            .checked_mul(output_k)
            .ok_or_else(|| invalid("CAGRA output shape overflow"))?;
        self.ensure_buffers(
            query_values.len(),
            pair_count,
            allowed.as_ref().map(|v| v.0.len()),
        )?;
        self.stream
            .memcpy_htod(&query_values, self.query.as_mut().expect("query buffer"))
            .map_err(cuda_error("query upload"))?;
        TELEMETRY.query_uploads.fetch_add(1, Ordering::Relaxed);
        TELEMETRY.h2d_bytes.fetch_add(
            (query_values.len() * size_of::<f32>()) as u64,
            Ordering::Relaxed,
        );

        let filtered = allowed.is_some();
        if let Some((words, _)) = allowed.as_ref() {
            self.stream
                .memcpy_htod(words, self.filter.as_mut().expect("filter buffer"))
                .map_err(cuda_error("filter upload"))?;
            TELEMETRY.filter_uploads.fetch_add(1, Ordering::Relaxed);
            TELEMETRY
                .h2d_bytes
                .fetch_add((words.len() * size_of::<u32>()) as u64, Ordering::Relaxed);
        }

        if filtered {
            self.launch_filtered_exact(metric, query_count, output_k)?;
            TELEMETRY
                .exact_filter_kernel_launches
                .fetch_add(1, Ordering::Relaxed);
        } else {
            self.search_params
                .configure(query_count, output_k, ef_search)?;
            let mut query_shape = [query_count as i64, self.dim as i64];
            let mut output_shape = [query_count as i64, output_k as i64];
            let mut distance_shape = output_shape;
            let query_view = self
                .query
                .as_ref()
                .expect("query buffer")
                .slice(..query_values.len());
            let mut id_view = self
                .ids
                .as_mut()
                .expect("id buffer")
                .slice_mut(..pair_count);
            let mut distance_view = self
                .distances
                .as_mut()
                .expect("distance buffer")
                .slice_mut(..pair_count);
            let (query_ptr, _query_guard) = query_view.device_ptr(&self.stream);
            let (id_ptr, _id_guard) = id_view.device_ptr_mut(&self.stream);
            let (distance_ptr, _distance_guard) = distance_view.device_ptr_mut(&self.stream);
            let mut query_tensor = device_tensor(query_ptr, &mut query_shape, dtype_f32());
            let mut id_tensor = device_tensor(id_ptr, &mut output_shape, dtype_i64());
            let mut distance_tensor = device_tensor(distance_ptr, &mut distance_shape, dtype_f32());
            check(
                unsafe {
                    ffi::cuvsCagraSearch(
                        self.resources.0,
                        self.search_params.0,
                        self.index.0,
                        &mut query_tensor,
                        &mut id_tensor,
                        &mut distance_tensor,
                        ffi::cuvsFilter {
                            addr: 0,
                            type_: ffi::cuvsFilterType::NO_FILTER,
                        },
                    )
                },
                "CAGRA search",
            )?;
            check(
                unsafe { ffi::cuvsStreamSync(self.resources.0) },
                "search sync",
            )?;
            drop((_query_guard, _id_guard, _distance_guard));
            TELEMETRY
                .cagra_kernel_launches
                .fetch_add(1, Ordering::Relaxed);
        }

        let host_ids = self
            .stream
            .clone_dtoh(&self.ids.as_ref().expect("id buffer").slice(..pair_count))
            .map_err(cuda_error("final id readback"))?;
        let host_distances = self
            .stream
            .clone_dtoh(
                &self
                    .distances
                    .as_ref()
                    .expect("distance buffer")
                    .slice(..pair_count),
            )
            .map_err(cuda_error("final distance readback"))?;
        TELEMETRY
            .final_readback_pairs
            .fetch_add(pair_count as u64, Ordering::Relaxed);
        TELEMETRY.d2h_bytes.fetch_add(
            (pair_count * (size_of::<i64>() + size_of::<f32>())) as u64,
            Ordering::Relaxed,
        );
        validate_output(
            host_ids,
            host_distances,
            query_count,
            output_k,
            self.rows,
            metric,
        )
    }

    fn ensure_buffers(
        &mut self,
        query_values: usize,
        pairs: usize,
        filter_words: Option<usize>,
    ) -> Result<()> {
        ensure(&self.stream, &mut self.query, query_values, "query")?;
        ensure(&self.stream, &mut self.ids, pairs, "neighbor")?;
        ensure(&self.stream, &mut self.distances, pairs, "distance")?;
        if let Some(words) = filter_words {
            ensure(&self.stream, &mut self.filter, words, "filter")?;
        }
        Ok(())
    }

    fn projected_bytes(&self, query_values: usize, pairs: usize, filter_words: usize) -> u64 {
        let query = projected_len(&self.query, query_values).saturating_mul(size_of::<f32>());
        let ids = projected_len(&self.ids, pairs).saturating_mul(size_of::<i64>());
        let distances = projected_len(&self.distances, pairs).saturating_mul(size_of::<f32>());
        let filter = projected_len(&self.filter, filter_words).saturating_mul(size_of::<u32>());
        self.base_bytes.saturating_add(
            u64::try_from(
                query
                    .saturating_add(ids)
                    .saturating_add(distances)
                    .saturating_add(filter),
            )
            .unwrap_or(u64::MAX),
        )
    }

    fn launch_filtered_exact(
        &mut self,
        metric: CagraServingMetric,
        query_count: usize,
        k: usize,
    ) -> Result<()> {
        let rows = i32::try_from(self.rows).map_err(|_| invalid("CAGRA rows exceed i32"))?;
        let dim = i32::try_from(self.dim).map_err(|_| invalid("CAGRA dim exceeds i32"))?;
        let queries = i32::try_from(query_count).map_err(|_| invalid("query count exceeds i32"))?;
        let metric = match metric {
            CagraServingMetric::UnitL2 => 1_i32,
            CagraServingMetric::RawL2 => 2_i32,
        };
        let k = i32::try_from(k).map_err(|_| invalid("CAGRA k exceeds i32"))?;
        let mut launch = self.stream.launch_builder(&self.exact);
        unsafe {
            launch
                .arg(&self.dataset_ptr)
                .arg(&rows)
                .arg(&dim)
                .arg(&self.dataset_row_stride)
                .arg(&self.dataset_column_stride)
                .arg(self.query.as_ref().expect("query buffer"))
                .arg(&queries)
                .arg(self.filter.as_ref().expect("filter buffer"))
                .arg(&metric)
                .arg(&k)
                .arg(self.ids.as_mut().expect("id buffer"))
                .arg(self.distances.as_mut().expect("distance buffer"))
                .launch(LaunchConfig {
                    grid_dim: (query_count as u32, 1, 1),
                    block_dim: (1, 1, 1),
                    shared_mem_bytes: 0,
                })
        }
        .map_err(cuda_error("filtered exact launch"))?;
        self.stream
            .synchronize()
            .map_err(cuda_error("filtered exact sync"))
    }
}

fn projected_len<T>(buffer: &Option<CudaSlice<T>>, requested: usize) -> usize {
    buffer
        .as_ref()
        .map_or(requested, |slice| slice.len().max(requested))
}

fn ensure<T>(
    stream: &Arc<CudaStream>,
    buffer: &mut Option<CudaSlice<T>>,
    len: usize,
    name: &'static str,
) -> Result<()>
where
    T: cudarc::driver::DeviceRepr + cudarc::driver::ValidAsZeroBits,
{
    if buffer.as_ref().is_none_or(|current| current.len() < len) {
        *buffer = Some(stream.alloc_zeros(len).map_err(cuda_error(name))?);
    }
    Ok(())
}

pub(super) fn check(status: ffi::cuvsError_t, stage: &'static str) -> Result<()> {
    if status == ffi::cuvsError_t::CUVS_SUCCESS {
        return Ok(());
    }
    TELEMETRY.failures.fetch_add(1, Ordering::Relaxed);
    let last = unsafe {
        let pointer = ffi::cuvsGetLastErrorText();
        if pointer.is_null() {
            "no cuVS error text".to_string()
        } else {
            CStr::from_ptr(pointer).to_string_lossy().into_owned()
        }
    };
    Err(unavailable(format!(
        "CAGRA serving {stage}: {status:?}; {last}"
    )))
}

fn cuda_error(
    stage: &'static str,
) -> impl FnOnce(cudarc::driver::DriverError) -> calyx_core::CalyxError {
    move |error| {
        TELEMETRY.failures.fetch_add(1, Ordering::Relaxed);
        unavailable(format!("CAGRA serving {stage}: {error}"))
    }
}

fn invalid(detail: impl Into<String>) -> calyx_core::CalyxError {
    sextant_error(CALYX_INDEX_INVALID_PARAMS, detail)
}

pub(super) fn unavailable(detail: impl Into<String>) -> calyx_core::CalyxError {
    sextant_error(CALYX_SEXTANT_GPU_SERVING_UNAVAILABLE, detail)
}
