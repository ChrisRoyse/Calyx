use std::fs;
use std::path::PathBuf;

use calyx_assay::{
    AssayCacheKey, AssayGate, AssayStore, AssaySubject, CoverageMask, MiEstimate, SlotAttribution,
    TrustTag, bits_report, bits_report_with_anchor, entropy_bits, ksg_mi_continuous_discrete,
    ksg_mi_continuous_discrete_with_anchor, logistic_probe_mi, logistic_probe_mi_with_anchor,
    panel_sufficiency, panel_sufficiency_with_anchor,
};
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{Anchor, AnchorKind, AnchorValue, SlotId, VaultId};
use serde_json::json;

#[test]
fn assay_trust_tags_follow_grounded_anchor_evidence() {
    let (samples, labels) = labeled_samples();
    let grounded = grounded_anchor();
    let ungrounded = ungrounded_anchor();
    let overconfident = overconfident_anchor();
    let discrete_labels: Vec<usize> = labels.iter().map(|label| usize::from(*label)).collect();
    let no_anchor = logistic_probe_mi(&samples, &labels).unwrap();
    let trusted = logistic_probe_mi_with_anchor(&samples, &labels, &grounded).unwrap();
    let provisional = logistic_probe_mi_with_anchor(&samples, &labels, &ungrounded).unwrap();
    let overconfident_provisional =
        logistic_probe_mi_with_anchor(&samples, &labels, &overconfident).unwrap();
    let ksg_no_anchor = ksg_mi_continuous_discrete(&samples, &discrete_labels, 3).unwrap();
    let ksg_trusted =
        ksg_mi_continuous_discrete_with_anchor(&samples, &discrete_labels, 3, &grounded).unwrap();
    let ksg_provisional =
        ksg_mi_continuous_discrete_with_anchor(&samples, &discrete_labels, 3, &ungrounded).unwrap();
    let gate = AssayGate::default();
    let gate_no_anchor = gate.lens_signal(&samples, &labels).unwrap();
    let gate_trusted = gate
        .lens_signal_with_anchor(&samples, &labels, &grounded)
        .unwrap();
    let right_samples = paired_samples(&samples);
    let gain_no_anchor = gate.pair_gain(&samples, &right_samples, &labels).unwrap();
    let gain_grounded = gate
        .pair_gain_with_anchor(&samples, &right_samples, &labels, &grounded)
        .unwrap();
    let pair_no_anchor = gate.pair_gain_estimate(&gain_no_anchor);
    let pair_trusted = gate.pair_gain_estimate_with_anchor(&gain_grounded, &grounded);
    let pair_provisional = gate.pair_gain_estimate_with_anchor(&gain_grounded, &ungrounded);
    let attributions = vec![
        SlotAttribution {
            slot: SlotId::new(1),
            marginal_bits: 0.31,
            sole_carrier: true,
            coverage: CoverageMask::Full,
        },
        SlotAttribution {
            slot: SlotId::new(2),
            marginal_bits: 0.04,
            sole_carrier: false,
            coverage: CoverageMask::Full,
        },
    ];
    let bits_no_anchor = bits_report(attributions.clone(), TrustTag::Trusted);
    let bits_trusted = bits_report_with_anchor(attributions.clone(), &grounded);
    let bits_provisional = bits_report_with_anchor(attributions.clone(), &ungrounded);
    let entropy = entropy_bits(&discrete_labels);
    let sufficiency_no_anchor = panel_sufficiency(0.35, entropy, &attributions, TrustTag::Trusted);
    let sufficiency_trusted =
        panel_sufficiency_with_anchor(0.35, entropy, &attributions, &grounded);
    let sufficiency_provisional =
        panel_sufficiency_with_anchor(0.35, entropy, &attributions, &ungrounded);

    assert_eq!(no_anchor.estimate.trust, TrustTag::Provisional);
    assert_eq!(trusted.estimate.trust, TrustTag::Trusted);
    assert_eq!(provisional.estimate.trust, TrustTag::Provisional);
    assert_eq!(
        overconfident_provisional.estimate.trust,
        TrustTag::Provisional
    );
    assert_eq!(ksg_no_anchor.trust, TrustTag::Provisional);
    assert_eq!(ksg_trusted.trust, TrustTag::Trusted);
    assert_eq!(ksg_provisional.trust, TrustTag::Provisional);
    assert_eq!(gate_no_anchor.estimate.trust, TrustTag::Provisional);
    assert_eq!(gate_trusted.estimate.trust, TrustTag::Trusted);
    assert_eq!(pair_no_anchor.trust, TrustTag::Provisional);
    assert_eq!(pair_trusted.trust, TrustTag::Trusted);
    assert_eq!(pair_provisional.trust, TrustTag::Provisional);
    assert_eq!(bits_no_anchor.trust, TrustTag::Provisional);
    assert_eq!(bits_trusted.trust, TrustTag::Trusted);
    assert_eq!(bits_provisional.trust, TrustTag::Provisional);
    assert_eq!(sufficiency_no_anchor.trust, TrustTag::Provisional);
    assert_eq!(sufficiency_trusted.trust, TrustTag::Trusted);
    assert_eq!(sufficiency_provisional.trust, TrustTag::Provisional);

    let rows = vec![
        ("logistic_no_anchor", no_anchor.estimate),
        ("logistic_grounded_anchor", trusted.estimate),
        ("logistic_ungrounded_anchor", provisional.estimate),
        (
            "logistic_overconfident_anchor",
            overconfident_provisional.estimate,
        ),
        ("ksg_no_anchor", ksg_no_anchor),
        ("ksg_grounded_anchor", ksg_trusted),
        ("ksg_ungrounded_anchor", ksg_provisional),
        ("pair_no_anchor", pair_no_anchor),
        ("pair_grounded_anchor", pair_trusted),
        ("pair_ungrounded_anchor", pair_provisional),
    ];
    let mut readback = persist_and_read_trust_rows(&rows);
    readback["report_trust"] = json!({
        "bits_no_anchor_requested_trusted": bits_no_anchor.trust,
        "bits_grounded_anchor": bits_trusted.trust,
        "bits_ungrounded_anchor": bits_provisional.trust,
        "sufficiency_no_anchor_requested_trusted": sufficiency_no_anchor.trust,
        "sufficiency_grounded_anchor": sufficiency_trusted.trust,
        "sufficiency_ungrounded_anchor": sufficiency_provisional.trust,
    });
    write_readback("assay-trust-readback.json", readback);
}

fn persist_and_read_trust_rows(rows: &[(&str, MiEstimate)]) -> serde_json::Value {
    let dir = fsv_root().join(format!("assay-trust-cf-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let mut router = CfRouter::open(&dir, 1_048_576).unwrap();
    let mut store = AssayStore::default();
    let key = AssayCacheKey::scoped(9, "trust-fsv", vault(), AnchorKind::Reward);
    let mut subjects = Vec::new();
    for (index, (name, estimate)) in rows.iter().enumerate() {
        let subject = AssaySubject::Lens {
            slot: SlotId::new((index + 1) as u16),
        };
        let provenance = match *name {
            name if name.ends_with("_no_anchor") => format!("case={name} anchor=absent"),
            name if name.ends_with("_ungrounded_anchor") => {
                format!("case={name} anchor=ungrounded source=empty confidence=0.0")
            }
            name if name.ends_with("_overconfident_anchor") => {
                format!("case={name} anchor=ungrounded source=synthetic-outcome confidence=1.1")
            }
            name if name.ends_with("_grounded_anchor") => {
                format!("case={name} anchor=grounded source=synthetic-outcome confidence=1.0")
            }
            name => format!("case={name} anchor=unknown"),
        };
        store.put(
            key.clone(),
            subject.clone(),
            estimate.clone(),
            provenance,
            (index + 1) as u64,
        );
        subjects.push(((*name).to_string(), subject));
    }
    let persisted_rows = store.persist_to_aster(&mut router).unwrap();
    let raw_cf_rows: Vec<_> = router
        .iter_cf(ColumnFamily::Assay)
        .unwrap()
        .into_iter()
        .map(|entry| {
            json!({
                "key_hex": hex(&entry.key),
                "value_len": entry.value.len(),
                "value_json": serde_json::from_slice::<serde_json::Value>(&entry.value).unwrap(),
            })
        })
        .collect();
    let loaded = AssayStore::load_from_aster(&router).unwrap();
    let mut loaded_trust = serde_json::Map::new();
    let mut loaded_provenance = serde_json::Map::new();
    for (name, subject) in subjects {
        let row = loaded.get(&key, &subject).unwrap();
        loaded_trust.insert(name.clone(), json!(row.estimate.trust));
        loaded_provenance.insert(name, json!(row.provenance));
    }

    json!({
        "source_of_truth": "Aster Assay CF rows after anchor-aware trust tagging",
        "cf_root": dir.join("cf/assay").display().to_string(),
        "persisted_rows": persisted_rows,
        "raw_cf_rows": raw_cf_rows,
        "loaded_trust": loaded_trust,
        "loaded_provenance": loaded_provenance,
    })
}

fn labeled_samples() -> (Vec<Vec<f32>>, Vec<bool>) {
    let mut samples = Vec::new();
    let mut labels = Vec::new();
    for i in 0..64 {
        let positive = i >= 32;
        labels.push(positive);
        if positive {
            samples.push(vec![1.0, 0.25 + i as f32 * 0.001]);
        } else {
            samples.push(vec![-1.0, -0.25 - i as f32 * 0.001]);
        }
    }
    (samples, labels)
}

fn paired_samples(samples: &[Vec<f32>]) -> Vec<Vec<f32>> {
    samples.iter().map(|row| vec![row[1]]).collect()
}

fn grounded_anchor() -> Anchor {
    Anchor {
        kind: AnchorKind::Reward,
        value: AnchorValue::Bool(true),
        source: "synthetic-outcome".to_string(),
        observed_at: 1_785_400_000,
        confidence: 1.0,
    }
}

fn ungrounded_anchor() -> Anchor {
    Anchor {
        kind: AnchorKind::Reward,
        value: AnchorValue::Bool(true),
        source: String::new(),
        observed_at: 1_785_400_001,
        confidence: 0.0,
    }
}

fn overconfident_anchor() -> Anchor {
    Anchor {
        kind: AnchorKind::Reward,
        value: AnchorValue::Bool(true),
        source: "synthetic-outcome".to_string(),
        observed_at: 1_785_400_002,
        confidence: 1.1,
    }
}

fn write_readback(name: &str, value: serde_json::Value) {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    println!("ASSAY_TRUST_READBACK={}", path.display());
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-assay-trust-fsv"))
}

fn vault() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
