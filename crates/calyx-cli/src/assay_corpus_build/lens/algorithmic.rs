use calyx_registry::{AlgorithmicLens, LensSpec as RegistryLensSpec};

pub(super) fn algorithmic_lens(
    spec: &RegistryLensSpec,
    kind: &str,
) -> Result<AlgorithmicLens, String> {
    AlgorithmicLens::from_runtime_kind(&spec.name, spec.modality, kind, spec.output).map_err(
        |error| {
            format!(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_ALGORITHMIC_RUNTIME: lens={} kind={kind}: {}: {}",
                spec.name, error.code, error.message
            )
        },
    )
}
