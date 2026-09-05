use std::collections::BTreeMap;

use super::runtime::load_runtime_lens;
use super::*;
use crate::lens::process_runtime_requires_golden;
use crate::{LensRuntime, LensSpec};

pub(crate) fn rebuild_registry(snapshot: &VaultRegistrySnapshot) -> Result<Registry> {
    let mut registry = Registry::new();
    for lens in &snapshot.lenses {
        validate_snapshot_identity(lens)?;
        let runtime: Arc<dyn Lens> = if lens
            .spec
            .as_ref()
            .is_some_and(runtime_requires_eager_reconstruction)
        {
            load_runtime_lens(lens)?
        } else {
            Arc::new(LazyPersistedLens::new(lens.clone()))
        };
        registry.register_persisted_arc(
            runtime,
            lens.contract.clone(),
            lens.spec.clone(),
            lens.determinism,
            lens.runtime_golden.clone(),
        )?;
    }
    Ok(registry)
}

pub(crate) fn rebuild_stored_vector_registry(snapshot: &VaultRegistrySnapshot) -> Result<Registry> {
    let mut registry = Registry::new();
    for lens in &snapshot.lenses {
        validate_snapshot_identity(lens)?;
        registry.register_persisted_arc(
            Arc::new(StoredVectorOnlyLens {
                snapshot: lens.clone(),
            }),
            lens.contract.clone(),
            lens.spec.clone(),
            lens.determinism,
            lens.runtime_golden.clone(),
        )?;
    }
    Ok(registry)
}

fn validate_snapshot_identity(lens: &RegistryLensSnapshot) -> Result<()> {
    if lens.lens_id != lens.contract.lens_id() {
        return Err(CalyxError::lens_frozen_violation(format!(
            "registry lens {} does not match frozen contract {}",
            lens.lens_id,
            lens.contract.lens_id()
        )));
    }
    Ok(())
}

fn runtime_requires_eager_reconstruction(spec: &LensSpec) -> bool {
    // Algorithmic runtimes may define input acceptance beyond the frozen
    // primary modality (structured bindings accept hybrid records). Rebuild
    // them now so the persisted registry preserves the real Lens contract.
    process_runtime_requires_golden(spec)
        || matches!(&spec.runtime, LensRuntime::Algorithmic { .. })
}

struct LazyPersistedLens {
    snapshot: RegistryLensSnapshot,
    runtime: Mutex<Option<LazyRuntimeCache>>,
}

enum LazyRuntimeCache {
    Loaded(Arc<dyn Lens>),
    Failed(String),
}

struct StoredVectorOnlyLens {
    snapshot: RegistryLensSnapshot,
}

impl Lens for StoredVectorOnlyLens {
    fn id(&self) -> LensId {
        self.snapshot.lens_id
    }

    fn shape(&self) -> SlotShape {
        self.snapshot.contract.shape()
    }

    fn modality(&self) -> Modality {
        self.snapshot.contract.modality()
    }

    fn measure(&self, _input: &Input) -> Result<SlotVector> {
        Err(CalyxError::lens_unreachable(format!(
            "lens {} was loaded with the stored-vectors-only capability; live measurement is forbidden",
            self.snapshot.lens_id
        )))
    }
}

impl LazyPersistedLens {
    fn new(snapshot: RegistryLensSnapshot) -> Self {
        Self {
            snapshot,
            runtime: Mutex::new(None),
        }
    }

    fn runtime(&self) -> Result<Arc<dyn Lens>> {
        let mut guard = self.runtime.lock().map_err(|_| {
            CalyxError::lens_unreachable(format!(
                "lazy persisted lens {} runtime mutex was poisoned",
                self.snapshot.lens_id
            ))
        })?;
        match guard.as_ref() {
            Some(LazyRuntimeCache::Loaded(runtime)) => return Ok(runtime.clone()),
            Some(LazyRuntimeCache::Failed(load_error)) => {
                return Err(self.error(load_error.clone()));
            }
            None => {}
        }
        match load_runtime_lens(&self.snapshot) {
            Ok(runtime) => {
                *guard = Some(LazyRuntimeCache::Loaded(runtime.clone()));
                Ok(runtime)
            }
            Err(error) => {
                let load_error = format!(
                    "{}: {} (remediation: {})",
                    error.code, error.message, error.remediation
                );
                *guard = Some(LazyRuntimeCache::Failed(load_error.clone()));
                Err(self.error(load_error))
            }
        }
    }

    fn error(&self, load_error: String) -> CalyxError {
        CalyxError::lens_unreachable(format!(
            "lens {} is persisted but its runtime failed to load in this process: {}",
            self.snapshot.lens_id, load_error
        ))
    }
}

impl Lens for LazyPersistedLens {
    fn id(&self) -> LensId {
        self.snapshot.lens_id
    }

    fn shape(&self) -> SlotShape {
        self.snapshot.contract.shape()
    }

    fn modality(&self) -> Modality {
        self.snapshot.contract.modality()
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        self.runtime()?.measure(input)
    }

    fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        self.runtime()?.measure_batch(inputs)
    }

    fn measurement_group_key(&self) -> Result<Option<calyx_core::MeasurementGroupKey>> {
        self.runtime()?.measurement_group_key()
    }

    fn measure_grouped_batch(
        &self,
        requests: &[calyx_core::GroupedLensRequest],
        inputs: &[Input],
    ) -> Result<Option<BTreeMap<LensId, Vec<SlotVector>>>> {
        self.runtime()?.measure_grouped_batch(requests, inputs)
    }
}
