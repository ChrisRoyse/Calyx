use std::collections::BTreeMap;
use std::sync::Arc;

use calyx_core::{Asymmetry, CalyxError, Input, Lens, LensId, Result, SlotVector};
use serde::{Deserialize, Serialize};

mod contract;
mod reproduce;
mod validation;

pub use validation::{ensure_input_modality, ensure_vector_shape};

use crate::drift::{PROCESS_RUNTIME_GOLDEN_TOLERANCE, RuntimeGolden};
use crate::frozen::FrozenLensContract;
use crate::ingest_microbatch::{IngestLensOutcome, IngestMicrobatchController, IngestPanelReadout};
use crate::spec::{LensHealth, LensRuntime, LensSpec};
use contract::ensure_spec_declares_contract;

const PROCESS_RUNTIME_GOLDEN_PROBE_BYTES: &[u8] = b"calyx frozen process runtime identity probe v1";

/// Runtime registry for frozen lens measurement instruments.
#[derive(Clone, Default)]
pub struct Registry {
    lenses: BTreeMap<LensId, RegistryEntry>,
}

#[derive(Clone)]
struct RegistryEntry {
    lens: Arc<dyn Lens>,
    frozen: Option<FrozenLensContract>,
    spec: Option<LensSpec>,
    determinism: DeterminismProof,
    runtime_golden: Option<RuntimeGolden>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FrozenLensSnapshot {
    pub lens_id: LensId,
    pub weights_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeterminismProof {
    ProbeVerified,
    ContractOnlyExemption,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RegistryLensSnapshot {
    pub lens_id: LensId,
    pub contract: FrozenLensContract,
    pub spec: Option<LensSpec>,
    pub determinism: DeterminismProof,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_golden: Option<RuntimeGolden>,
}

impl Registry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fails closed: runtime lenses must be registered with a frozen contract.
    pub fn register<L>(&mut self, _lens: L) -> Result<LensId>
    where
        L: Lens + 'static,
    {
        Err(CalyxError::lens_frozen_violation(
            "Registry::register requires register_frozen with a FrozenLensContract",
        ))
    }

    /// Fails closed: structured metadata does not replace a frozen contract.
    pub fn register_with_spec<L>(&mut self, _lens: L, _spec: LensSpec) -> Result<LensId>
    where
        L: Lens + 'static,
    {
        Err(CalyxError::lens_frozen_violation(
            "Registry::register_with_spec requires register_frozen_with_spec with a FrozenLensContract",
        ))
    }

    /// Registers a lens and enforces its frozen content-addressed contract.
    pub fn register_frozen<L>(&mut self, lens: L, contract: FrozenLensContract) -> Result<LensId>
    where
        L: Lens + 'static,
    {
        self.register_frozen_inner(lens, contract, None, None)
    }

    /// Registers a frozen lens with structured registry metadata.
    pub fn register_frozen_with_spec<L>(
        &mut self,
        lens: L,
        contract: FrozenLensContract,
        spec: LensSpec,
    ) -> Result<LensId>
    where
        L: Lens + 'static,
    {
        self.register_frozen_inner(lens, contract, None, Some(spec))
    }

    /// Registers a process-boundary lens with a caller-supplied identity probe.
    pub fn register_frozen_with_spec_and_probe<L>(
        &mut self,
        lens: L,
        contract: FrozenLensContract,
        spec: LensSpec,
        probe: &Input,
    ) -> Result<LensId>
    where
        L: Lens + 'static,
    {
        self.register_frozen_inner(lens, contract, Some(probe), Some(spec))
    }

    /// Registers an already-constructed frozen lens with structured registry metadata.
    pub fn register_frozen_arc_with_spec(
        &mut self,
        lens: Arc<dyn Lens>,
        contract: FrozenLensContract,
        spec: LensSpec,
    ) -> Result<LensId> {
        self.register_frozen_arc_inner(lens, contract, None, Some(spec))
    }

    /// Registers a frozen lens after a deterministic two-pass probe.
    pub fn register_frozen_with_probe<L>(
        &mut self,
        lens: L,
        contract: FrozenLensContract,
        probe: &Input,
    ) -> Result<LensId>
    where
        L: Lens + 'static,
    {
        self.register_frozen_inner(lens, contract, Some(probe), None)
    }

    /// Returns true when a lens id is registered.
    pub fn contains(&self, id: LensId) -> bool {
        self.lenses.contains_key(&id)
    }

    /// Finds a registered lens by its stable frozen/spec name.
    pub fn find_lens_by_name(&self, name: &str) -> Option<LensId> {
        self.lenses
            .iter()
            .find(|(_, entry)| {
                entry.spec.as_ref().is_some_and(|spec| spec.name == name)
                    || entry
                        .frozen
                        .as_ref()
                        .is_some_and(|contract| contract.name() == name)
            })
            .map(|(lens_id, _)| *lens_id)
    }

    /// Finds a registered runtime lens by the content-addressed LensSpec id.
    pub fn find_lens_by_spec_id(&self, spec_lens_id: LensId) -> Option<LensId> {
        self.lenses
            .iter()
            .find(|(_, entry)| {
                entry
                    .spec
                    .as_ref()
                    .is_some_and(|spec| spec.lens_id() == spec_lens_id)
            })
            .map(|(lens_id, _)| *lens_id)
    }

    /// Measures one input with a registered lens.
    pub fn measure(&self, lens_id: LensId, input: &Input) -> Result<SlotVector> {
        let entry = self.lookup(lens_id)?;
        ensure_input_modality(entry.lens.as_ref(), input)?;
        let vector = entry.lens.measure(input)?;
        self.validate_entry(lens_id, entry, &vector)?;
        Ok(vector)
    }

    /// Measures a batch with a registered lens and validates every result.
    pub fn measure_batch(&self, lens_id: LensId, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        let entry = self.lookup(lens_id)?;
        for input in inputs {
            ensure_input_modality(entry.lens.as_ref(), input)?;
        }

        let vectors = entry.lens.measure_batch(inputs)?;
        if vectors.len() != inputs.len() {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "lens {lens_id} returned {} vectors for {} inputs",
                vectors.len(),
                inputs.len()
            )));
        }
        for vector in &vectors {
            self.validate_entry(lens_id, entry, vector)?;
        }
        Ok(vectors)
    }

    /// Measures an ingest microbatch across lenses with bounded admission and degradation.
    pub fn measure_ingest_microbatch(
        &self,
        lens_ids: &[LensId],
        inputs: &[Input],
        admission: &IngestMicrobatchController,
        now_ms: u64,
    ) -> Result<IngestPanelReadout> {
        let mut outcomes = Vec::with_capacity(lens_ids.len());
        for &lens_id in lens_ids {
            self.lookup(lens_id)?;
            let outcome: IngestLensOutcome =
                admission.measure_lens_batch(lens_id, inputs, now_ms, |batch| {
                    self.measure_batch(lens_id, batch)
                })?;
            outcomes.push(outcome);
        }
        Ok(admission.panel_readout(inputs.len(), outcomes))
    }

    /// Measures both directions of an asymmetric dual lens.
    pub fn measure_dual(&self, lens_id: LensId, input: &Input) -> Result<DualMeasurement> {
        let entry = self.lookup(lens_id)?;
        let asymmetry = entry
            .spec
            .as_ref()
            .map(|spec| spec.asymmetry)
            .unwrap_or(Asymmetry::None);
        if !matches!(asymmetry, Asymmetry::Dual { .. }) {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "lens {lens_id} is not registered as a dual-direction lens"
            )));
        }
        let _ = input;
        Err(CalyxError::lens_unreachable(format!(
            "lens {lens_id} declares dual asymmetry but registry has no directional runtime; refusing byte-reversed surrogate"
        )))
    }

    /// Returns the frozen contract registered for a lens id.
    pub fn frozen_contract(&self, lens_id: LensId) -> Option<&FrozenLensContract> {
        self.lenses
            .get(&lens_id)
            .and_then(|entry| entry.frozen.as_ref())
    }

    /// Returns all registered frozen lens weight hashes in stable id order.
    pub fn frozen_lens_snapshots(&self) -> Vec<FrozenLensSnapshot> {
        self.lenses
            .iter()
            .filter_map(|(lens_id, entry)| {
                entry.frozen.as_ref().map(|contract| FrozenLensSnapshot {
                    lens_id: *lens_id,
                    weights_sha256: contract.weights_sha256(),
                })
            })
            .collect()
    }

    /// Returns structured metadata for a lens id, when registered.
    pub fn lens_spec(&self, lens_id: LensId) -> Option<&LensSpec> {
        self.lenses
            .get(&lens_id)
            .and_then(|entry| entry.spec.as_ref())
    }

    pub fn lens_snapshots(&self) -> Vec<RegistryLensSnapshot> {
        self.lenses
            .iter()
            .filter_map(|(lens_id, entry)| {
                entry.frozen.as_ref().map(|contract| RegistryLensSnapshot {
                    lens_id: *lens_id,
                    contract: contract.clone(),
                    spec: entry.spec.clone(),
                    determinism: entry.determinism,
                    runtime_golden: entry.runtime_golden.clone(),
                })
            })
            .collect()
    }

    pub(crate) fn register_persisted_arc(
        &mut self,
        lens: Arc<dyn Lens>,
        contract: FrozenLensContract,
        spec: Option<LensSpec>,
        determinism: DeterminismProof,
        runtime_golden: Option<RuntimeGolden>,
    ) -> Result<LensId> {
        contract.verify_registration(lens.as_ref())?;
        if let Some(spec) = &spec {
            ensure_spec_declares_contract(&contract, spec)?;
        }
        if let Some(golden) = &runtime_golden {
            verify_runtime_golden_identity(&contract, golden)?;
        }
        let id = lens.id();
        if self.lenses.contains_key(&id) {
            return Err(CalyxError::registry_duplicate(format!(
                "lens {id} is already registered"
            )));
        }
        self.lenses.insert(
            id,
            RegistryEntry {
                lens,
                frozen: Some(contract),
                spec,
                determinism,
                runtime_golden,
            },
        );
        Ok(id)
    }

    /// Returns whether registration verified a deterministic probe or used an explicit exemption.
    pub fn determinism_proof(&self, lens_id: LensId) -> Option<DeterminismProof> {
        self.lenses.get(&lens_id).map(|entry| entry.determinism)
    }

    /// Probes runtime health for a registered lens.
    pub fn health(&self, lens_id: LensId) -> Result<LensHealth> {
        let entry = self.lookup(lens_id)?;
        Ok(entry
            .spec
            .as_ref()
            .map(LensSpec::health)
            .unwrap_or(LensHealth::Loaded))
    }

    fn register_frozen_inner<L>(
        &mut self,
        lens: L,
        contract: FrozenLensContract,
        probe: Option<&Input>,
        spec: Option<LensSpec>,
    ) -> Result<LensId>
    where
        L: Lens + 'static,
    {
        self.register_frozen_arc_inner(Arc::new(lens), contract, probe, spec)
    }

    fn register_frozen_arc_inner(
        &mut self,
        lens: Arc<dyn Lens>,
        contract: FrozenLensContract,
        probe: Option<&Input>,
        spec: Option<LensSpec>,
    ) -> Result<LensId> {
        contract.verify_registration(lens.as_ref())?;
        if let Some(spec) = &spec {
            ensure_spec_declares_contract(&contract, spec)?;
        }

        let runtime_version = spec.as_ref().and_then(process_runtime_golden_version);
        let default_probe = runtime_version.map(|_| {
            Input::new(
                contract.modality(),
                PROCESS_RUNTIME_GOLDEN_PROBE_BYTES.to_vec(),
            )
        });
        let effective_probe = probe.or(default_probe.as_ref());
        let verified_output = effective_probe
            .map(|probe| contract.measure_determinism_probe(lens.as_ref(), probe))
            .transpose()?;
        let runtime_golden = match (runtime_version, effective_probe, verified_output.as_ref()) {
            (Some(runtime_version), Some(probe), Some(output)) => {
                let golden_output = output.as_dense().ok_or_else(|| {
                    CalyxError::lens_frozen_violation(
                        "process runtime identity probes require dense output",
                    )
                })?;
                Some(RuntimeGolden {
                    lens_id: contract.lens_id(),
                    runtime_version: runtime_version.to_string(),
                    probe: probe.clone(),
                    golden_output: golden_output.to_vec(),
                    tolerance: PROCESS_RUNTIME_GOLDEN_TOLERANCE,
                })
            }
            _ => None,
        };
        let determinism = if effective_probe.is_some() {
            DeterminismProof::ProbeVerified
        } else {
            DeterminismProof::ContractOnlyExemption
        };
        let id = lens.id();
        if self.lenses.contains_key(&id) {
            return Err(CalyxError::registry_duplicate(format!(
                "lens {id} is already registered"
            )));
        }
        self.lenses.insert(
            id,
            RegistryEntry {
                lens,
                frozen: Some(contract),
                spec,
                determinism,
                runtime_golden,
            },
        );
        Ok(id)
    }

    fn validate_entry(
        &self,
        lens_id: LensId,
        entry: &RegistryEntry,
        vector: &SlotVector,
    ) -> Result<()> {
        if let Some(contract) = &entry.frozen {
            contract.verify_registration(entry.lens.as_ref())?;
            contract.verify_vector(lens_id, vector)
        } else {
            ensure_vector_shape(lens_id, entry.lens.shape(), vector)
        }
    }

    fn lookup(&self, lens_id: LensId) -> Result<&RegistryEntry> {
        self.lenses.get(&lens_id).ok_or_else(|| {
            CalyxError::lens_unreachable(format!("lens {lens_id} is not registered"))
        })
    }
}

pub(crate) fn process_runtime_requires_golden(spec: &LensSpec) -> bool {
    process_runtime_golden_version(spec).is_some()
}

fn process_runtime_golden_version(spec: &LensSpec) -> Option<&'static str> {
    match &spec.runtime {
        LensRuntime::TeiHttp { .. } => Some("tei-http-golden-v1"),
        LensRuntime::ExternalCmd { .. } => Some("external-cmd-golden-v1"),
        _ => None,
    }
}

fn verify_runtime_golden_identity(
    contract: &FrozenLensContract,
    golden: &RuntimeGolden,
) -> Result<()> {
    if golden.lens_id != contract.lens_id() {
        return Err(CalyxError::lens_frozen_violation(format!(
            "runtime golden lens {} does not match frozen contract {}",
            golden.lens_id,
            contract.lens_id()
        )));
    }
    if golden.probe.modality != contract.modality() {
        return Err(CalyxError::lens_frozen_violation(format!(
            "runtime golden probe modality {:?} does not match frozen {:?}",
            golden.probe.modality,
            contract.modality()
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DualMeasurement {
    pub a: SlotVector,
    pub b: SlotVector,
}

#[cfg(test)]
mod tests;
