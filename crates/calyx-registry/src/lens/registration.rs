use super::*;

impl Registry {
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

    pub(super) fn register_frozen_inner<L>(
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

    pub(super) fn register_frozen_arc_inner(
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
                let golden_output = process_runtime_golden_vector(output).ok_or_else(|| {
                    CalyxError::lens_frozen_violation(
                        "process runtime identity probes require dense output or exactly one native segmented vector",
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

    pub(super) fn validate_entry(
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

    pub(super) fn lookup(&self, lens_id: LensId) -> Result<&RegistryEntry> {
        self.lenses.get(&lens_id).ok_or_else(|| {
            CalyxError::lens_unreachable(format!("lens {lens_id} is not registered"))
        })
    }
}
