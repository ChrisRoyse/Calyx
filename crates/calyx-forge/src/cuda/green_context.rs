use std::mem::MaybeUninit;
use std::ptr;

use cudarc::driver::{result, sys};

use crate::{ForgeError, Result};

use super::{CudaContext, init_cuda};

const GREEN_CONTEXT_REMEDIATION: &str = "Unset CALYX_ONNX_GREEN_CONTEXT_SMS, lower the requested SM count, or verify CUDA 13.3 green-context driver support";

#[derive(Debug)]
pub struct CudaGreenContextStream {
    _primary: CudaContext,
    green_ctx: sys::CUgreenCtx,
    stream: sys::CUstream,
    green_ctx_id: u64,
    requested_sm_count: u32,
    actual_sm_count: u32,
    total_sm_count: u32,
    workqueue_balanced: bool,
}

unsafe impl Send for CudaGreenContextStream {}

impl CudaGreenContextStream {
    pub fn create_serving(device_idx: u32, requested_sm_count: u32) -> Result<Self> {
        if requested_sm_count == 0 {
            return Err(green_context_error(
                "requested_sm_count must be greater than zero",
            ));
        }
        let primary = init_cuda(device_idx, false)?;
        let priority = serving_priority(&primary)?;
        create_stream(primary, requested_sm_count, priority)
    }

    pub fn stream_ptr(&self) -> *mut () {
        self.stream.cast()
    }

    pub const fn green_ctx_id(&self) -> u64 {
        self.green_ctx_id
    }

    pub const fn requested_sm_count(&self) -> u32 {
        self.requested_sm_count
    }

    pub const fn actual_sm_count(&self) -> u32 {
        self.actual_sm_count
    }

    pub const fn total_sm_count(&self) -> u32 {
        self.total_sm_count
    }

    pub const fn workqueue_balanced(&self) -> bool {
        self.workqueue_balanced
    }
}

impl Drop for CudaGreenContextStream {
    fn drop(&mut self) {
        let stream = std::mem::replace(&mut self.stream, ptr::null_mut());
        if !stream.is_null() {
            let _ = unsafe { sys::cuStreamDestroy_v2(stream).result() };
        }
        let green_ctx = std::mem::replace(&mut self.green_ctx, ptr::null_mut());
        if !green_ctx.is_null() {
            let _ = unsafe { sys::cuGreenCtxDestroy(green_ctx).result() };
        }
    }
}

fn create_stream(
    primary: CudaContext,
    requested_sm_count: u32,
    priority: i32,
) -> Result<CudaGreenContextStream> {
    primary
        .inner()
        .bind_to_thread()
        .map_err(driver_error("bind primary context"))?;
    let device = result::device::get(primary.device_idx() as i32)
        .map_err(driver_error("get CUDA device"))?;
    let device_sm = device_sm_resource(device)?;
    let total_sm_count = sm_count(&device_sm);
    if requested_sm_count > total_sm_count {
        return Err(green_context_error(format!(
            "requested_sm_count={requested_sm_count} exceeds total_sm_count={total_sm_count}"
        )));
    }

    let mut selected_sm = zeroed_resource();
    let mut remaining_sm = zeroed_resource();
    let mut groups = 1_u32;
    unsafe {
        sys::cuDevSmResourceSplitByCount(
            &mut selected_sm,
            &mut groups,
            &device_sm,
            &mut remaining_sm,
            0,
            requested_sm_count,
        )
        .result()
    }
    .map_err(driver_error("split SM resources"))?;
    if groups == 0 {
        return Err(green_context_error(
            "CUDA returned zero SM resource groups for green context",
        ));
    }

    let mut resources = vec![selected_sm, balanced_workqueue_resource(device)?];
    let mut desc: sys::CUdevResourceDesc = ptr::null_mut();
    unsafe {
        sys::cuDevResourceGenerateDesc(&mut desc, resources.as_mut_ptr(), resources.len() as u32)
            .result()
    }
    .map_err(driver_error("generate resource descriptor"))?;
    if desc.is_null() {
        return Err(green_context_error(
            "CUDA returned a null green-context resource descriptor",
        ));
    }

    let mut green_ctx: sys::CUgreenCtx = ptr::null_mut();
    unsafe { sys::cuGreenCtxCreate(&mut green_ctx, desc, device, 0).result() }
        .map_err(driver_error("create green context"))?;
    if green_ctx.is_null() {
        return Err(green_context_error("CUDA returned a null green context"));
    }

    let mut actual_sm = zeroed_resource();
    let actual_sm_count = unsafe {
        match sys::cuGreenCtxGetDevResource(
            green_ctx,
            &mut actual_sm,
            sys::CUdevResourceType::CU_DEV_RESOURCE_TYPE_SM,
        )
        .result()
        {
            Ok(()) => sm_count(&actual_sm),
            Err(err) => {
                let _ = sys::cuGreenCtxDestroy(green_ctx).result();
                return Err(driver_error("query green context SM resources")(err));
            }
        }
    };

    let mut green_ctx_id = 0_u64;
    unsafe {
        if let Err(err) = sys::cuGreenCtxGetId(green_ctx, &mut green_ctx_id).result() {
            let _ = sys::cuGreenCtxDestroy(green_ctx).result();
            return Err(driver_error("query green context id")(err));
        }
    }

    let mut stream: sys::CUstream = ptr::null_mut();
    unsafe {
        if let Err(err) = sys::cuGreenCtxStreamCreate(
            &mut stream,
            green_ctx,
            sys::CUstream_flags::CU_STREAM_NON_BLOCKING as u32,
            priority,
        )
        .result()
        {
            let _ = sys::cuGreenCtxDestroy(green_ctx).result();
            return Err(driver_error("create green context stream")(err));
        }
    }
    if stream.is_null() {
        unsafe {
            let _ = sys::cuGreenCtxDestroy(green_ctx).result();
        }
        return Err(green_context_error(
            "CUDA returned a null green-context stream",
        ));
    }

    Ok(CudaGreenContextStream {
        _primary: primary,
        green_ctx,
        stream,
        green_ctx_id,
        requested_sm_count,
        actual_sm_count,
        total_sm_count,
        workqueue_balanced: true,
    })
}

fn serving_priority(ctx: &CudaContext) -> Result<i32> {
    ctx.inner()
        .bind_to_thread()
        .map_err(driver_error("bind primary context"))?;
    let (_least_priority, greatest_priority) = result::stream::get_priority_range()
        .map_err(driver_error("query stream priority range"))?;
    Ok(greatest_priority)
}

fn device_sm_resource(device: sys::CUdevice) -> Result<sys::CUdevResource> {
    let mut resource = zeroed_resource();
    unsafe {
        sys::cuDeviceGetDevResource(
            device,
            &mut resource,
            sys::CUdevResourceType::CU_DEV_RESOURCE_TYPE_SM,
        )
        .result()
    }
    .map_err(driver_error("query device SM resources"))?;
    Ok(resource)
}

fn balanced_workqueue_resource(device: sys::CUdevice) -> Result<sys::CUdevResource> {
    let mut resource = zeroed_resource();
    unsafe {
        sys::cuDeviceGetDevResource(
            device,
            &mut resource,
            sys::CUdevResourceType::CU_DEV_RESOURCE_TYPE_WORKQUEUE_CONFIG,
        )
        .result()
    }
    .map_err(driver_error("query device workqueue resources"))?;
    resource.__bindgen_anon_1.wqConfig.sharingScope =
        sys::CUdevWorkqueueConfigScope::CU_WORKQUEUE_SCOPE_GREEN_CTX_BALANCED;
    Ok(resource)
}

fn zeroed_resource() -> sys::CUdevResource {
    unsafe { MaybeUninit::<sys::CUdevResource>::zeroed().assume_init() }
}

fn sm_count(resource: &sys::CUdevResource) -> u32 {
    unsafe { resource.__bindgen_anon_1.sm.smCount }
}

fn driver_error(stage: &'static str) -> impl FnOnce(result::DriverError) -> ForgeError {
    move |err| green_context_error(format!("CUDA green-context {stage} failed: {err}"))
}

fn green_context_error(detail: impl Into<String>) -> ForgeError {
    ForgeError::GpuError {
        detail: detail.into(),
        remediation: GREEN_CONTEXT_REMEDIATION.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn green_context_stream_can_initialize() -> Result<()> {
        let stream = CudaGreenContextStream::create_serving(0, 8)?;

        println!(
            "CUDA_GREEN_CONTEXT id={} requested_sm_count={} actual_sm_count={} total_sm_count={} workqueue_balanced={}",
            stream.green_ctx_id(),
            stream.requested_sm_count(),
            stream.actual_sm_count(),
            stream.total_sm_count(),
            stream.workqueue_balanced()
        );

        assert!(!stream.stream_ptr().is_null());
        assert_eq!(stream.requested_sm_count(), 8);
        assert!(stream.actual_sm_count() >= 8);
        assert!(stream.actual_sm_count() <= stream.total_sm_count());
        assert!(stream.workqueue_balanced());
        Ok(())
    }
}
