use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Input, Lens, Modality, SlotVector};
use calyx_registry::{
    CALYX_LICENSE_DENIED, MultimodalAdapterLens, MultimodalAdapterSpec, MultimodalAxis,
    ProfileProbe, Registry, default_multimodal_lens_specs, profile_lens,
    register_multimodal_lens_pack,
};
use serde_json::json;

#[test]
fn issue788_multimodal_lens_pack_fsv_readback() {
    let (root, keep_root) = fsv_root();
    let cards_dir = root.join("cards");
    fs::create_dir_all(&cards_dir).unwrap();

    let mut registry = Registry::new();
    let entries =
        register_multimodal_lens_pack(&mut registry, &default_multimodal_lens_specs()).unwrap();
    let mut measurements = Vec::new();
    for entry in &entries {
        let axis = MultimodalAxis::from_modality(entry.spec.modality).unwrap();
        let vector = registry.measure(entry.lens_id, &happy_input(axis)).unwrap();
        let (dim, norm, prefix) = dense_readback(&vector);
        let probes = profile_inputs(axis)
            .into_iter()
            .map(ProfileProbe::new)
            .collect::<Vec<_>>();
        let card = profile_lens(&registry, entry.lens_id, &probes).unwrap();
        let card_path = cards_dir.join(format!("{}.json", axis.as_str()));
        fs::write(&card_path, serde_json::to_vec_pretty(&card).unwrap()).unwrap();
        measurements.push(json!({
            "axis": axis.as_str(),
            "modality": format!("{:?}", entry.spec.modality).to_ascii_lowercase(),
            "lens_id": entry.lens_id.to_string(),
            "dim": dim,
            "norm": norm,
            "first_values": prefix,
            "card_path": card_path,
            "card_probe_count": card.probe_count,
            "card_measured": card.coverage.measured,
            "health": card.health,
        }));
    }

    let edges = malformed_edges();
    let license = license_gate_readback();
    fs::write(
        root.join("registry-snapshot.json"),
        serde_json::to_vec_pretty(&registry.lens_snapshots()).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("measurements.json"),
        serde_json::to_vec_pretty(&measurements).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("edges.json"),
        serde_json::to_vec_pretty(&edges).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("license.json"),
        serde_json::to_vec_pretty(&license).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("summary.json"),
        serde_json::to_vec_pretty(&json!({
            "issue": 788,
            "registered_lenses": entries.len(),
            "measurement_rows": measurements.len(),
            "edge_rows": edges.len(),
            "license_denied_code": license["denied"]["error_code"],
        }))
        .unwrap(),
    )
    .unwrap();

    assert_eq!(entries.len(), 5);
    assert!(measurements.iter().all(|row| row["dim"] == 16));
    assert!(
        edges
            .iter()
            .all(|row| row["after"]["error_code"] == "CALYX_LENS_DIM_MISMATCH")
    );
    assert_eq!(license["denied"]["error_code"], CALYX_LICENSE_DENIED);

    if !keep_root {
        let _ = fs::remove_dir_all(root);
    }
}

fn fsv_root() -> (PathBuf, bool) {
    if let Some(root) = std::env::var_os("CALYX_FSV_ROOT") {
        return (PathBuf::from(root), true);
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    (
        std::env::temp_dir().join(format!("calyx-issue788-fsv-{}-{nanos}", std::process::id())),
        false,
    )
}

fn happy_input(axis: MultimodalAxis) -> Input {
    profile_inputs(axis).remove(0)
}

fn profile_inputs(axis: MultimodalAxis) -> Vec<Input> {
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

fn malformed_edges() -> Vec<serde_json::Value> {
    [
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
    ]
    .into_iter()
    .map(|(axis, input)| {
        let lens =
            MultimodalAdapterLens::from_adapter_spec(adapter_spec(axis, None, false)).unwrap();
        let error = lens.measure(&input).unwrap_err();
        json!({
            "axis": axis.as_str(),
            "before": {
                "input_len": input.bytes.len(),
                "attempted": false,
            },
            "after": {
                "attempted": true,
                "error_code": error.code,
                "error_message": error.message,
            }
        })
    })
    .collect()
}

fn license_gate_readback() -> serde_json::Value {
    let denied = MultimodalAdapterLens::from_adapter_spec(adapter_spec(
        MultimodalAxis::Dna,
        Some("CC-BY-NC-SA-4.0"),
        false,
    ))
    .unwrap_err();
    let allowed = MultimodalAdapterLens::from_adapter_spec(adapter_spec(
        MultimodalAxis::Dna,
        Some("CC-BY-NC-SA-4.0"),
        true,
    ))
    .unwrap();
    json!({
        "denied": {
            "license": "CC-BY-NC-SA-4.0",
            "allow_flag": false,
            "error_code": denied.code,
            "error_message": denied.message,
        },
        "allowed": {
            "license": "CC-BY-NC-SA-4.0",
            "allow_flag": true,
            "modality": format!("{:?}", allowed.modality()).to_ascii_lowercase(),
        }
    })
}

fn adapter_spec(
    axis: MultimodalAxis,
    license: Option<&str>,
    allow_non_commercial: bool,
) -> MultimodalAdapterSpec {
    MultimodalAdapterSpec {
        name: format!("issue788-{}", axis.as_str()),
        axis,
        model_id: format!("fixture/{}", axis.as_str()),
        dim: 16,
        license: license.map(str::to_string),
        allow_non_commercial,
    }
}

fn dense_readback(vector: &SlotVector) -> (u32, f32, Vec<f32>) {
    let SlotVector::Dense { dim, data } = vector else {
        panic!("expected dense vector");
    };
    let norm = data.iter().map(|value| value * value).sum::<f32>().sqrt();
    (*dim, norm, data.iter().take(4).copied().collect())
}
