use super::OnnxProviderPolicy;

pub(super) struct CudaDropGuard<T> {
    value: Option<T>,
    leak_on_drop: bool,
}

impl<T> CudaDropGuard<T> {
    pub(super) fn new(value: T, provider_policy: OnnxProviderPolicy) -> Self {
        Self {
            value: Some(value),
            leak_on_drop: provider_policy == OnnxProviderPolicy::CudaFailLoud,
        }
    }

    pub(super) fn as_ref(&self) -> &T {
        self.value
            .as_ref()
            .expect("CudaDropGuard value is present until into_inner")
    }

    pub(super) fn into_inner(mut self) -> T {
        self.value
            .take()
            .expect("CudaDropGuard value is present until into_inner")
    }
}

impl<T> Drop for CudaDropGuard<T> {
    fn drop(&mut self) {
        if self.leak_on_drop
            && let Some(value) = self.value.take()
        {
            // ORT CUDA provider teardown can corrupt glibc heap in a manual verification run.
            // Successful lenses leak in OnnxLens::drop; this guard applies the
            // same policy to construction errors after a CUDA session exists.
            std::mem::forget(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    struct DropProbe(Arc<AtomicUsize>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn cuda_guard_leaks_cuda_fail_loud_error_path() {
        let drops = Arc::new(AtomicUsize::new(0));

        {
            let _guard = CudaDropGuard::new(
                DropProbe(Arc::clone(&drops)),
                OnnxProviderPolicy::CudaFailLoud,
            );
        }

        assert_eq!(drops.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn cuda_guard_drops_cpu_explicit_error_path() {
        let drops = Arc::new(AtomicUsize::new(0));

        {
            let _guard = CudaDropGuard::new(
                DropProbe(Arc::clone(&drops)),
                OnnxProviderPolicy::CpuExplicit,
            );
        }

        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cuda_guard_into_inner_transfers_ownership() {
        let drops = Arc::new(AtomicUsize::new(0));

        let probe = CudaDropGuard::new(
            DropProbe(Arc::clone(&drops)),
            OnnxProviderPolicy::CudaFailLoud,
        )
        .into_inner();
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        drop(probe);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}
