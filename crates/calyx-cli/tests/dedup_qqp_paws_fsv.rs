//! PH70 / issue #605 FSV: TCT cosine-Gtau dedup correctness on real QQP/PAWS.
//!
//! Gate (PRD `28 section 2`, `28 section 3` row 5): dedup merges true duplicates at a
//! calibrated tau and NEVER merges conflicting anchors - proven on the real
//! labeled near-duplicate corpora with byte readback of the persisted vault.

use std::env;
use std::fs;
use std::path::PathBuf;

use calyx_aster::dedup::{
    CALYX_DEDUP_ANCHOR_CONFLICT, CALYX_DEDUP_DPI_EXCEEDED, CALYX_DEDUP_INVALID_TAU,
    CALYX_DEDUP_NO_REQUIRED_SLOTS, CALYX_DEDUP_SLOT_NOT_IN_CONSTELLATION, DedupAction, DedupResult,
    EpochSecs, TauStrategy, TctCosineConfig, check_dedup_with_limit, dedup_audit, ingest_at,
};
use calyx_core::{Lens, Modality, dense_cosine};
use calyx_registry::TeiHttpLens;
use serde_json::json;

#[path = "support/dedup_qqp_paws_io.rs"]
mod io;

use io::{
    Confusion, PairRow, calibrate_tau, confusion_at_tau, contested_readback, durable_vault,
    engine_pair_decision, label_anchor, label_means, merged, parse_pairs_tsv, probe_constellation,
    text_input, vector_at_cos, write_blake3_sums, write_json,
};

const TEI_DIM: u32 = 768;
const PRECISION_FLOOR: f64 = 0.95;
const EVAL_PRECISION_GATE: f64 = 0.85;
const EVAL_RECALL_GATE: f64 = 0.20;
const SEPARATION_GATE: f64 = 0.10;
const PAWS_WOULD_MERGE_GATE: usize = 50;
const ANCHOR_GUARD_PAIRS: usize = 16;
const COMPATIBLE_CONTROL_PAIRS: usize = 8;

#[test]
fn parse_synthetic_pairs_known_io() {
    let tsv = "source\tsplit\tpair_id\tlabel\ta\tb\ttext_a\ttext_b\n\
        qqp\tcalib\t1\t1\tx\tx\thow do I learn rust\thow can I learn rust\n\
        qqp\tcalib\t2\t1\tx\tx\twhat is gravity\twhat is gravity exactly\n\
        qqp\teval\t3\t1\tx\tx\tbest pizza in rome\twhere is the best pizza in rome\n\
        qqp\teval\t4\t0\tx\tx\thow do I learn rust\thow do I learn french\n\
        paws\tadversarial\t5\t0\tx\tx\tflights from NY to FL\tflights from FL to NY\n\
        paws\tadversarial\t6\t0\tx\tx\tA beat B\tB beat A\n";
    let rows = parse_pairs_tsv(tsv).expect("parse synthetic tsv");
    assert_eq!(rows.len(), 6);
    assert_eq!(rows.iter().filter(|row| row.label == 1).count(), 3);
    assert_eq!(rows.iter().filter(|row| row.label == 0).count(), 3);
    assert_eq!(
        parse_pairs_tsv("h\nqqp\tcalib\t1\t2\tx\tx\ta\tb\n").unwrap_err(),
        "line 2 label \"2\" not 0/1"
    );
    assert!(parse_pairs_tsv("header only\n").is_err());
}

#[test]
fn calibration_known_io() {
    // Hand-computed: dups at {0.9, 0.8, 0.7}, non-dups at {0.6, 0.5}.
    // Smallest tau with precision 1.0 is 0.7 -> recall 3/3.
    let clean = [(0.9, 1), (0.8, 1), (0.7, 1), (0.6, 0), (0.5, 0)];
    let (tau, precision, recall) = calibrate_tau(&clean, 1.0).expect("calibrates");
    assert_eq!(tau, 0.7);
    assert_eq!(precision, 1.0);
    assert_eq!(recall, 1.0);
    // Overlap: dups {0.9, 0.6}, non-dup {0.7}. Floor 0.9 forces tau=0.9 -> recall 1/2.
    let overlap = [(0.9, 1), (0.6, 1), (0.7, 0)];
    let (tau, precision, recall) = calibrate_tau(&overlap, 0.9).expect("calibrates");
    assert_eq!(tau, 0.9);
    assert_eq!(precision, 1.0);
    assert_eq!(recall, 0.5);
    // Impossible floor: every threshold admits the 0.95 non-dup before any dup.
    assert_eq!(calibrate_tau(&[(0.9, 1), (0.95, 0)], 0.9), None);
    // Confusion at tau=0.7 on `clean`: tp=3 fp=0 fn=0 tn=2 (2+2=4 discipline).
    let confusion = confusion_at_tau(&clean, 0.7);
    assert_eq!(
        confusion,
        Confusion {
            tp: 3,
            fp: 0,
            fn_: 0,
            tn: 2
        }
    );
}

#[test]
fn engine_near_threshold_and_temporal_edges() {
    let root = temp_root("edges");
    // tau=0.90; cos 0.89 must stay New, cos 0.91 must merge, identical bytes at
    // the same event time are ExactDuplicate. Vectors are exact unit vectors.
    let vault = durable_vault(&root.join("near"), 0.90, DedupAction::Collapse);
    let base = text_input("edge-base", vector_at_cos(1.0));
    ingest_at(&vault, &base, EpochSecs(100), None).expect("ingest base");
    let below = ingest_at(
        &vault,
        &text_input("edge-below", vector_at_cos(0.89)),
        EpochSecs(200),
        None,
    )
    .expect("ingest below");
    assert!(
        matches!(below, DedupResult::New(_)),
        "0.89 < tau: {below:?}"
    );
    let above = ingest_at(
        &vault,
        &text_input("edge-above", vector_at_cos(0.91)),
        EpochSecs(300),
        None,
    )
    .expect("ingest above");
    assert!(merged(&above), "0.91 >= tau must merge: {above:?}");
    let exact = ingest_at(&vault, &base, EpochSecs(100), None).expect("ingest exact");
    assert!(
        matches!(exact, DedupResult::ExactDuplicate(_)),
        "identical bytes at same event time: {exact:?}"
    );
    // Temporal-only difference under RecurrenceSeries: same content, new
    // event time -> occurrence appended to the same region, not a new region.
    let series = durable_vault(&root.join("series"), 0.90, DedupAction::RecurrenceSeries);
    let recurring = text_input("recurring-event", vector_at_cos(1.0));
    let first = ingest_at(&series, &recurring, EpochSecs(100), None).expect("series first");
    let second = ingest_at(&series, &recurring, EpochSecs(200), None).expect("series second");
    let DedupResult::New(into) = first else {
        panic!("first series ingest must be New: {first:?}")
    };
    let DedupResult::DedupMerge {
        into: merged_into,
        occurrence,
    } = second
    else {
        panic!("temporal-only difference must append an occurrence: {second:?}")
    };
    assert_eq!(merged_into, into);
    assert_eq!(occurrence.0, 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fail_closed_codes_are_exact() {
    let root = temp_root("fail-closed");
    assert_eq!(
        TctCosineConfig::new(
            vec![io::slot(0)],
            TauStrategy::PerSlot(vec![(io::slot(0), 1.5)]),
            DedupAction::Collapse,
        )
        .unwrap_err()
        .code,
        CALYX_DEDUP_INVALID_TAU
    );
    assert_eq!(
        TctCosineConfig::new(Vec::new(), TauStrategy::Calibrated, DedupAction::Collapse)
            .unwrap_err()
            .code,
        CALYX_DEDUP_NO_REQUIRED_SLOTS
    );
    let vault = durable_vault(&root.join("vault"), 0.9, DedupAction::Collapse);
    ingest_at(
        &vault,
        &text_input("seed-a", vector_at_cos(1.0)),
        EpochSecs(100),
        None,
    )
    .expect("seed a");
    // Required slot missing from the new constellation: fail closed.
    let missing_slot = calyx_aster::dedup::IngestInput::new(
        b"no-slot".to_vec(),
        io::PANEL_VERSION,
        Modality::Text,
    );
    assert_eq!(
        ingest_at(&vault, &missing_slot, EpochSecs(150), None)
            .unwrap_err()
            .code,
        CALYX_DEDUP_SLOT_NOT_IN_CONSTELLATION
    );
    // DPI candidate-limit breach: fail closed instead of silently scanning.
    ingest_at(
        &vault,
        &text_input("seed-b", vector_at_cos(-1.0)),
        EpochSecs(200),
        None,
    )
    .expect("seed b");
    let probe = text_input("probe", vector_at_cos(0.0));
    let policy = io::tct_policy(0.9, DedupAction::Collapse);
    let cx = probe_constellation(&vault, &probe);
    assert_eq!(
        check_dedup_with_limit(&cx, &vault, &policy, None, 1)
            .unwrap_err()
            .code,
        CALYX_DEDUP_DPI_EXCEEDED
    );
    // Conflicting anchor on an exact duplicate: hard error, never a merge.
    let anchored =
        text_input("anchored", vector_at_cos(0.5)).with_anchor(label_anchor("axis", "left", "fsv"));
    ingest_at(&vault, &anchored, EpochSecs(300), None).expect("anchored seed");
    let conflicting = text_input("anchored", vector_at_cos(0.5))
        .with_anchor(label_anchor("axis", "right", "fsv"));
    assert_eq!(
        ingest_at(&vault, &conflicting, EpochSecs(300), None)
            .unwrap_err()
            .code,
        CALYX_DEDUP_ANCHOR_CONFLICT
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
#[ignore = "manual FSV: real QQP/PAWS pairs + resident TEI :8088 + durable-vault byte readback"]
fn qqp_paws_dedup_intelligence_fsv() {
    let out = PathBuf::from(require_env("CALYX_FSV_OUT"));
    fs::create_dir_all(&out).expect("create FSV out root");
    let pairs_path = env::var("CALYX_DEDUP_PAIRS_TSV")
        .unwrap_or_else(|_| "/zfs/archive/calyx/datasets/dedup_fsv_pairs.tsv".to_string());
    let text = fs::read_to_string(&pairs_path).unwrap_or_else(|error| {
        panic!("CALYX_DATASET_MISSING: {pairs_path}: {error} - run scripts/acquire_dedup.sh first")
    });
    let rows = parse_pairs_tsv(&text).expect("parse acquired pairs TSV");

    // Embed every unique text once through the resident TEI lens.
    let lens = TeiHttpLens::resident_8088("qqp-paws-dedup-fsv", TEI_DIM)
        .with_timeout(std::time::Duration::from_secs(120))
        .with_max_batch(64);
    let mut texts: Vec<&str> = Vec::new();
    for row in &rows {
        for side in [row.text_a.as_str(), row.text_b.as_str()] {
            if !texts.contains(&side) {
                texts.push(side);
            }
        }
    }
    let inputs: Vec<calyx_core::Input> = texts
        .iter()
        .map(|text| calyx_core::Input::new(Modality::Text, text.as_bytes().to_vec()))
        .collect();
    let vectors = lens.measure_batch(&inputs).expect("TEI embeddings");
    let dense = |text: &str| -> Vec<f32> {
        let index = texts.iter().position(|t| *t == text).expect("known text");
        let calyx_core::SlotVector::Dense { data, .. } = &vectors[index] else {
            panic!("TEI returned non-dense vector")
        };
        data.clone()
    };
    let cos_of = |row: &PairRow| -> f32 {
        dense_cosine(&dense(&row.text_a), &dense(&row.text_b)).expect("finite cosine")
    };

    // Calibrate tau on the QQP calibration split (precision-first).
    let calib: Vec<(f32, u8)> = rows
        .iter()
        .filter(|row| row.source == "qqp" && row.split == "calib")
        .map(|row| (cos_of(row), row.label))
        .collect();
    let (tau, calib_precision, calib_recall) =
        calibrate_tau(&calib, PRECISION_FLOOR).expect("calibration must find a feasible tau");

    // Evaluate every QQP eval pair through the REAL engine (durable vaults).
    let vault_root = out.join("vaults");
    fs::create_dir_all(&vault_root).expect("create vault root");
    let eval_rows: Vec<&PairRow> = rows
        .iter()
        .filter(|row| row.source == "qqp" && row.split == "eval")
        .collect();
    let mut confusion = Confusion::default();
    let mut eval_cosines = Vec::new();
    let mut per_slot_readback = None;
    for (index, row) in eval_rows.iter().enumerate() {
        let cos = cos_of(row);
        eval_cosines.push((cos, row.label));
        let dir = vault_root.join(format!("eval_{index}"));
        let (decision, vault) =
            engine_pair_decision(&dir, tau, dense(&row.text_a), dense(&row.text_b), row, None)
                .expect("engine eval ingest");
        let did_merge = merged(&decision);
        assert_eq!(
            did_merge,
            cos >= tau,
            "engine/math parity broke on pair {} (cos {cos}, tau {tau}): {decision:?}",
            row.pair_id
        );
        confusion.observe(did_merge, row.label);
        // Per-slot cosine byte readback on the first true-positive merge.
        if per_slot_readback.is_none()
            && did_merge
            && row.label == 1
            && let DedupResult::DedupMerge { into, .. } = &decision
        {
            let audit = dedup_audit(&vault, *into).expect("dedup audit");
            let (slot_id, stored_cos) = audit.merges[0].per_slot_cos[0];
            assert_eq!(slot_id, io::slot(io::CONTENT_SLOT));
            assert!(
                (stored_cos - cos).abs() <= 1e-6,
                "ledger per-slot cos {stored_cos} != TEI cos {cos}"
            );
            per_slot_readback = Some(json!({
                "pair_id": row.pair_id,
                "vault_dir": dir,
                "into": into,
                "tei_cos": cos,
                "ledger_per_slot_cos": stored_cos,
                "merges": audit.merges.len(),
            }));
            continue; // keep this vault on disk as evidence
        }
        let _ = fs::remove_dir_all(&dir);
    }
    let precision = confusion.precision();
    let recall = confusion.recall();
    let means = label_means(&eval_cosines);
    let separation = means[&1] - means[&0];

    // PAWS adversarial: cosine alone would merge most labeled non-duplicates.
    let paws: Vec<(&PairRow, f32)> = rows
        .iter()
        .filter(|row| row.source == "paws")
        .map(|row| (row, cos_of(row)))
        .collect();
    let mut would_merge: Vec<&(&PairRow, f32)> = paws
        .iter()
        .filter(|(row, cos)| row.label == 0 && *cos >= tau)
        .collect();
    would_merge.sort_by(|left, right| right.1.total_cmp(&left.1));
    let would_merge_count = would_merge.len();

    // NEVER-merge clause: conflicting anchors block the highest-cosine
    // adversarial non-duplicates that cosine alone would have merged.
    let mut guard_results = Vec::new();
    let guard_pairs: Vec<&(&PairRow, f32)> = would_merge
        .iter()
        .filter(|(row, _)| row.text_a != row.text_b)
        .take(ANCHOR_GUARD_PAIRS)
        .copied()
        .collect();
    assert_eq!(
        guard_pairs.len(),
        ANCHOR_GUARD_PAIRS,
        "need {ANCHOR_GUARD_PAIRS} distinct-text adversarial pairs"
    );
    for (index, (row, cos)) in guard_pairs.iter().enumerate() {
        let dir = vault_root.join(format!("guard_{index}"));
        let axis = format!("paws_pair_{}", row.pair_id);
        let source = "paws:labeled_final:test label=0";
        let (decision, vault) = engine_pair_decision(
            &dir,
            tau,
            dense(&row.text_a),
            dense(&row.text_b),
            row,
            Some((
                label_anchor(&axis, "side-a-claim", source),
                label_anchor(&axis, "side-b-claim", source),
            )),
        )
        .expect("guard ingest");
        assert!(
            matches!(decision, DedupResult::New(_)),
            "conflicting anchors MUST block merge (pair {}, cos {cos}): {decision:?}",
            row.pair_id
        );
        guard_results.push(contested_readback(&vault, &dir, row, *cos, index < 4));
        if index >= 4 {
            let _ = fs::remove_dir_all(&dir);
        }
    }

    // Compatible-anchor control: identical anchors never block a true merge.
    let mut control_results = Vec::new();
    let dup_pairs: Vec<&(&PairRow, f32)> = paws
        .iter()
        .filter(|(row, cos)| row.label == 1 && *cos >= tau && row.text_a != row.text_b)
        .take(COMPATIBLE_CONTROL_PAIRS)
        .collect();
    assert_eq!(dup_pairs.len(), COMPATIBLE_CONTROL_PAIRS);
    for (index, (row, cos)) in dup_pairs.iter().enumerate() {
        let dir = vault_root.join(format!("control_{index}"));
        let axis = format!("paws_pair_{}", row.pair_id);
        let source = "paws:labeled_final:test label=1";
        let (decision, _vault) = engine_pair_decision(
            &dir,
            tau,
            dense(&row.text_a),
            dense(&row.text_b),
            row,
            Some((
                label_anchor(&axis, "shared-claim", source),
                label_anchor(&axis, "shared-claim", source),
            )),
        )
        .expect("control ingest");
        assert!(
            merged(&decision),
            "compatible anchors must not block merge (pair {}, cos {cos}): {decision:?}",
            row.pair_id
        );
        control_results.push(json!({"pair_id": row.pair_id, "cos": cos, "merged": true}));
        let _ = fs::remove_dir_all(&dir);
    }

    write_json(
        &out.join("ph70_qqp_dedup.json"),
        &json!({
            "pairs_tsv": pairs_path,
            "tei": {"endpoint": "http://127.0.0.1:8088/embed", "model": "Alibaba-NLP/gte-multilingual-base", "dim": TEI_DIM},
            "trigger": "calibrate tau on qqp calib split, evaluate qqp eval split through real engine ingests",
            "calibration": {"precision_floor": PRECISION_FLOOR, "tau": tau, "precision": calib_precision, "recall": calib_recall, "pairs": calib.len()},
            "eval": {"confusion": confusion, "precision": precision, "recall": recall,
                      "mean_cos_dup": means[&1], "mean_cos_nondup": means[&0], "separation": separation,
                      "gates": {"precision": EVAL_PRECISION_GATE, "recall": EVAL_RECALL_GATE, "separation": SEPARATION_GATE}},
            "per_slot_cos_readback": per_slot_readback,
        }),
    );
    write_json(
        &out.join("ph70_paws_anchor_guard.json"),
        &json!({
            "trigger": "ingest highest-cosine labeled non-duplicate PAWS pairs with label-grounded conflicting anchors",
            "expected": "engine returns New + contested_with rows in Online CF; zero merges",
            "paws_pairs": paws.len(),
            "nondup_would_merge_at_tau": would_merge_count,
            "would_merge_gate": PAWS_WOULD_MERGE_GATE,
            "anchor_guard": guard_results,
            "compatible_control": control_results,
        }),
    );
    write_blake3_sums(&out);

    assert!(
        precision >= EVAL_PRECISION_GATE,
        "eval precision {precision} < {EVAL_PRECISION_GATE}"
    );
    assert!(
        recall >= EVAL_RECALL_GATE,
        "eval recall {recall} < {EVAL_RECALL_GATE}"
    );
    assert!(
        separation >= SEPARATION_GATE,
        "separation {separation} < {SEPARATION_GATE}"
    );
    assert!(
        would_merge_count >= PAWS_WOULD_MERGE_GATE,
        "adversarial premise vacuous: only {would_merge_count} PAWS non-dups >= tau"
    );
    println!(
        "FSV OK tau={tau} precision={precision:.4} recall={recall:.4} separation={separation:.4} \
         paws_would_merge={would_merge_count} guard_blocked={}/{} evidence={}",
        guard_results.len(),
        ANCHOR_GUARD_PAIRS,
        out.display()
    );
}

fn require_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("{name} must be set for the manual FSV run"))
}

fn temp_root(tag: &str) -> PathBuf {
    let root = env::temp_dir().join(format!("calyx-dedup-qqp-paws-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp root");
    root
}
