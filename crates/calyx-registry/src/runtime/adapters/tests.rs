use calyx_core::{Input, Lens, Modality, SlotVector};

use super::{
    CALYX_LICENSE_DENIED, MultimodalAdapterLens, MultimodalAdapterSpec, MultimodalAxis,
    default_multimodal_lens_specs, register_multimodal_lens_pack,
};
use crate::{LensHealth, ProfileProbe, Registry, profile_lens};

#[test]
fn multimodal_pack_registers_measures_unit_vectors_and_profiles() {
    let mut registry = Registry::new();
    let entries =
        register_multimodal_lens_pack(&mut registry, &default_multimodal_lens_specs()).unwrap();

    assert_eq!(entries.len(), 5);
    for entry in entries {
        let axis = MultimodalAxis::from_modality(entry.spec.modality).unwrap();
        let probes = probes(axis);
        let vector = registry.measure(entry.lens_id, &probes[0]).unwrap();
        assert_dense_unit(&vector, 16);

        let card_probes = probes
            .into_iter()
            .map(ProfileProbe::new)
            .collect::<Vec<_>>();
        let card = profile_lens(&registry, entry.lens_id, &card_probes).unwrap();
        assert_eq!(card.probe_count, 2);
        assert_eq!(card.coverage.measured, 2);
        assert_eq!(card.health, LensHealth::Loaded);

        let reloaded = MultimodalAdapterLens::from_lens_spec(&entry.spec).unwrap();
        entry.contract.verify_registration(&reloaded).unwrap();
    }
}

#[test]
fn license_gate_denies_noncommercial_by_default_and_allows_explicit_flag() {
    let denied = MultimodalAdapterLens::from_adapter_spec(adapter_spec(
        "nc-dna",
        MultimodalAxis::Dna,
        Some("CC-BY-NC-SA-4.0"),
        false,
    ))
    .unwrap_err();

    assert_eq!(denied.code, CALYX_LICENSE_DENIED);

    let allowed = MultimodalAdapterLens::from_adapter_spec(adapter_spec(
        "nc-dna",
        MultimodalAxis::Dna,
        Some("CC-BY-NC-SA-4.0"),
        true,
    ))
    .unwrap();
    assert_eq!(allowed.modality(), Modality::Dna);
}

#[test]
fn malformed_inputs_return_typed_errors_without_panic() {
    let cases = [
        (
            MultimodalAxis::Image,
            Input::new(Modality::Image, b"not-an-image".to_vec()),
        ),
        (
            MultimodalAxis::Audio,
            Input::new(Modality::Audio, b"RIFFbad".to_vec()),
        ),
        (
            MultimodalAxis::Protein,
            Input::new(Modality::Protein, b"ACDZ".to_vec()),
        ),
        (
            MultimodalAxis::Dna,
            Input::new(Modality::Dna, b"ACGTX".to_vec()),
        ),
        (
            MultimodalAxis::Molecule,
            Input::new(Modality::Molecule, b"C?O".to_vec()),
        ),
    ];

    for (axis, input) in cases {
        let lens = MultimodalAdapterLens::from_adapter_spec(adapter_spec("bad", axis, None, false))
            .unwrap();
        let error = lens.measure(&input).unwrap_err();
        assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");
        assert!(error.message.contains(axis.as_str()));
    }
}

fn adapter_spec(
    name: &str,
    axis: MultimodalAxis,
    license: Option<&str>,
    allow_non_commercial: bool,
) -> MultimodalAdapterSpec {
    MultimodalAdapterSpec {
        name: name.to_string(),
        axis,
        model_id: format!("fixture/{}", axis.as_str()),
        dim: 16,
        license: license.map(str::to_string),
        allow_non_commercial,
    }
}

fn probes(axis: MultimodalAxis) -> Vec<Input> {
    match axis {
        MultimodalAxis::Image => vec![
            Input::new(Modality::Image, b"\x89PNG\r\n\x1a\ncalyx-image-a".to_vec()),
            Input::new(Modality::Image, vec![0xff, 0xd8, 0xff, b'b']),
        ],
        MultimodalAxis::Audio => vec![
            Input::new(Modality::Audio, b"RIFF\x24\x00\x00\x00WAVEfmt a".to_vec()),
            Input::new(Modality::Audio, b"RIFF\x28\x00\x00\x00WAVEfmt b".to_vec()),
        ],
        MultimodalAxis::Protein => vec![
            Input::new(Modality::Protein, b"MTEYKLVVVG".to_vec()),
            Input::new(Modality::Protein, b"GAGGVGKSAL".to_vec()),
        ],
        MultimodalAxis::Dna => vec![
            Input::new(Modality::Dna, b"ACGTACGTNN".to_vec()),
            Input::new(Modality::Dna, b"TTGACCGTAA".to_vec()),
        ],
        MultimodalAxis::Molecule => vec![
            Input::new(Modality::Molecule, b"CCO".to_vec()),
            Input::new(Modality::Molecule, b"c1ccccc1".to_vec()),
        ],
    }
}

fn assert_dense_unit(vector: &SlotVector, expected_dim: u32) {
    let SlotVector::Dense { dim, data } = vector else {
        panic!("expected dense vector");
    };
    assert_eq!(*dim, expected_dim);
    assert_eq!(data.len(), expected_dim as usize);
    assert!(data.iter().all(|value| value.is_finite()));
    let norm = data.iter().map(|value| value * value).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1.0e-5, "{norm}");
}
