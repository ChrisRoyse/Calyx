//! PH68 T06 - beamwidth/posting-cutoff autotune tests (issue #550).

use std::path::{Path, PathBuf};

use calyx_core::FixedClock;
use calyx_sextant::index::{
    BwPostcutoffAnnealRegistry, BwPostcutoffConfig, BwPostcutoffTuner, TuneDirection, TunerConfig,
    TunerObservation, TunerRange, build_synthetic_vault, register_with_anneal,
};
use calyx_sextant::{CALYX_ANNEAL_UNAVAILABLE, TunerAdjustmentKind};
use proptest::prelude::*;
use serde::Serialize;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("calyx-autotune-t06")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn initial() -> BwPostcutoffConfig {
    BwPostcutoffConfig {
        beamwidth: 64,
        posting_cutoff: 1024,
    }
}

fn obs(latency: u64, recall: f32, config: BwPostcutoffConfig) -> TunerObservation {
    TunerObservation {
        query_latency_us: latency,
        recall_at_10: recall,
        beamwidth: config.beamwidth,
        posting_cutoff: config.posting_cutoff,
    }
}

#[test]
fn good_window_within_slo_does_not_revert() {
    let mut tuner = BwPostcutoffTuner::new(initial());
    for _ in 0..512 {
        tuner.observe(obs(20_000, 0.92, initial()));
    }

    let adjustment = tuner.maybe_adjust();

    assert!(adjustment.is_none());
    assert!(tuner.ledger_entries().is_empty());
}

#[test]
fn tripwire_reverts_and_logs_ledger_entry() {
    let mut tuner = BwPostcutoffTuner::new(initial());
    for _ in 0..462 {
        tuner.observe(obs(20_000, 0.92, initial()));
    }
    let bad = BwPostcutoffConfig {
        beamwidth: 32,
        posting_cutoff: 512,
    };
    for _ in 0..50 {
        tuner.observe(obs(19_000, 0.80, bad));
    }

    let adjustment = tuner.maybe_adjust().expect("tripwire revert");

    assert_eq!(adjustment.kind, TunerAdjustmentKind::Revert);
    assert_eq!(adjustment.old, bad);
    assert_eq!(adjustment.new, initial());
    assert_eq!(tuner.ledger_entries()[0].event, "diskann_tuner_revert");
    assert_eq!(tuner.ledger_entries()[0].reason, "recall_below_floor");
}

#[test]
fn anti_oscillation_blocks_direction_flip_inside_window() {
    let config = TunerConfig {
        beamwidth: TunerRange::new(8, 512, 8),
        posting_cutoff: TunerRange::new(64, 65_536, 64),
        ..TunerConfig::default()
    };
    let mut tuner = BwPostcutoffTuner::with_config(initial(), config);
    for _ in 0..512 {
        tuner.observe(obs(40_000, 0.93, initial()));
    }
    let first = tuner.maybe_adjust().expect("first adjustment");
    assert_eq!(first.direction, Some(TuneDirection::BeamwidthDown));

    let beam_at_min = BwPostcutoffConfig {
        beamwidth: 8,
        posting_cutoff: 1024,
    };
    for _ in 0..20 {
        tuner.observe(obs(41_000, 0.93, beam_at_min));
    }

    assert!(tuner.maybe_adjust().is_none());
}

#[test]
fn zero_observations_returns_none() {
    let mut tuner = BwPostcutoffTuner::new(initial());
    assert!(tuner.maybe_adjust().is_none());
}

#[test]
fn posting_cutoff_below_min_is_clamped() {
    let config = TunerConfig {
        beamwidth: TunerRange::new(8, 512, 8),
        posting_cutoff: TunerRange::new(128, 4096, 64),
        ..TunerConfig::default()
    };
    let current = BwPostcutoffConfig {
        beamwidth: 8,
        posting_cutoff: 128,
    };
    let tuner = BwPostcutoffTuner::with_config(current, config);

    let preview = tuner.preview_direction(TuneDirection::PostingCutoffDown);

    assert_eq!(preview.posting_cutoff, 128);
}

#[test]
fn unavailable_anneal_registry_warns_and_keeps_standalone_adjustments() {
    let tuner = BwPostcutoffTuner::new(initial());
    let mut tuner = register_with_anneal::<CountingRegistry>(tuner, None);
    for _ in 0..512 {
        tuner.observe(obs(40_000, 0.93, initial()));
    }

    let adjustment = tuner.maybe_adjust().expect("standalone proposal");

    assert!(tuner.is_standalone());
    assert_eq!(tuner.warnings()[0].code, CALYX_ANNEAL_UNAVAILABLE);
    assert_eq!(tuner.adjustment_history(), &[adjustment]);
}

#[test]
fn injected_clock_builds_latency_observation_without_wall_clock() {
    let clock = FixedClock::new(1_000);
    let observation = TunerObservation::from_clock(&clock, 997, 0.91, 64, 1024);

    assert_eq!(observation.query_latency_us, 3000);
}

#[test]
fn synthetic_vault_helper_writes_diskann_and_spann_files() {
    let root = scratch("vault-helper");
    let vault = build_synthetic_vault(128, 8, 1, 550, &root).expect("vault");

    assert!(vault.root.join("idx/slot_00.ann/graph.cda").is_file());
    assert!(
        vault
            .root
            .join("idx/slot_00.sparse/centroids.spn")
            .is_file()
    );
    assert_eq!(vault.rows.len(), 128);
}

#[test]
#[ignore = "server-only FSV trigger writes tuner state readback artifacts"]
fn fsv_issue550_writes_tuner_readback() {
    let root = fsv_root("CALYX_AUTOTUNE_FSV_DIR");
    let before = if root.join("tuner-readback.json").exists() {
        "EXISTS"
    } else {
        "MISSING"
    };
    std::fs::create_dir_all(&root).expect("create fsv root");
    std::fs::write(root.join("before.txt"), before).expect("write before");

    let mut tuner = BwPostcutoffTuner::new(initial());
    for _ in 0..512 {
        tuner.observe(obs(40_000, 0.93, initial()));
    }
    let adjustment = tuner.maybe_adjust().expect("proposal");
    let vault_files = vault_readback(&root);
    let report = TunerReadback {
        trigger: "bw_postcutoff_window_full",
        expected: "latency above 25000us proposes lower resource config",
        current: tuner.current_config(),
        adjustment,
        vault_files,
        ledger_entries: tuner.ledger_entries().to_vec(),
        warnings: tuner.warnings().to_vec(),
    };
    std::fs::write(
        root.join("tuner-readback.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .expect("write readback");
}

#[test]
#[ignore = "server-only FSV trigger writes tuner edge artifacts"]
fn fsv_issue550_edges_write_before_after_artifacts() {
    let root = fsv_root("CALYX_AUTOTUNE_EDGE_DIR");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create edge root");
    edge_no_observations(&root);
    edge_clamp(&root);
    edge_unavailable_registry(&root);
    edge_recall_revert(&root);
}

fn edge_no_observations(root: &Path) {
    let mut tuner = BwPostcutoffTuner::new(initial());
    std::fs::write(root.join("empty-before.txt"), "observations=0\n").unwrap();
    let result = tuner.maybe_adjust();
    std::fs::write(root.join("empty-after.txt"), "observations=0\n").unwrap();
    std::fs::write(root.join("empty-result.txt"), format!("{result:?}\n")).unwrap();
}

fn edge_clamp(root: &Path) {
    let config = TunerConfig {
        posting_cutoff: TunerRange::new(128, 4096, 64),
        ..TunerConfig::default()
    };
    let current = BwPostcutoffConfig {
        beamwidth: 8,
        posting_cutoff: 128,
    };
    let tuner = BwPostcutoffTuner::with_config(current, config);
    std::fs::write(root.join("clamp-before.txt"), "posting_cutoff=128\n").unwrap();
    let preview = tuner.preview_direction(TuneDirection::PostingCutoffDown);
    std::fs::write(root.join("clamp-after.txt"), "posting_cutoff=128\n").unwrap();
    std::fs::write(
        root.join("clamp-result.txt"),
        format!("proposed_posting_cutoff={}\n", preview.posting_cutoff),
    )
    .unwrap();
}

fn edge_unavailable_registry(root: &Path) {
    let tuner = BwPostcutoffTuner::new(initial());
    std::fs::write(root.join("registry-before.txt"), "warnings=0\n").unwrap();
    let tuner = register_with_anneal::<CountingRegistry>(tuner, None);
    std::fs::write(
        root.join("registry-after.txt"),
        format!("warnings={}\n", tuner.warnings().len()),
    )
    .unwrap();
    std::fs::write(root.join("registry-result.txt"), tuner.warnings()[0].code).unwrap();
}

fn edge_recall_revert(root: &Path) {
    let mut tuner = BwPostcutoffTuner::new(initial());
    for _ in 0..462 {
        tuner.observe(obs(20_000, 0.92, initial()));
    }
    let bad = BwPostcutoffConfig {
        beamwidth: 32,
        posting_cutoff: 512,
    };
    std::fs::write(root.join("revert-before.txt"), format!("{bad:?}\n")).unwrap();
    for _ in 0..50 {
        tuner.observe(obs(19_000, 0.80, bad));
    }
    let adjustment = tuner.maybe_adjust().unwrap();
    std::fs::write(
        root.join("revert-after.txt"),
        format!("{:?}\n", adjustment.new),
    )
    .unwrap();
    std::fs::write(
        root.join("revert-result.txt"),
        tuner.ledger_entries()[0].event.clone(),
    )
    .unwrap();
}

fn fsv_root(name: &str) -> PathBuf {
    let root = std::env::var(name).map(PathBuf::from).expect("set FSV dir");
    assert!(root.to_string_lossy().contains("issue550"));
    root
}

fn vault_readback(root: &Path) -> Vec<FileReadback> {
    let vault_root = root.join("synthetic-vault");
    let vault = build_synthetic_vault(256, 8, 1, 550, &vault_root).expect("build fsv vault");
    let mut files = vec![
        readback_file(&vault.root, "idx/slot_00.ann/graph.cda"),
        readback_file(&vault.root, "idx/slot_00.sparse/centroids.spn"),
    ];
    let mut postings = std::fs::read_dir(vault.root.join("idx/slot_00.sparse"))
        .expect("read sparse dir")
        .filter_map(|entry| {
            let entry = entry.expect("sparse entry");
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".spb")
                .then(|| {
                    let path =
                        format!("idx/slot_00.sparse/{}", entry.file_name().to_string_lossy());
                    readback_file(&vault.root, &path)
                })
        })
        .collect::<Vec<_>>();
    postings.sort_by(|a, b| a.path.cmp(&b.path));
    files.extend(postings);
    files
}

fn readback_file(root: &Path, relative: &str) -> FileReadback {
    let path = root.join(relative);
    FileReadback {
        path: relative.to_string(),
        size: std::fs::metadata(path).expect("read metadata").len(),
    }
}

#[derive(Default)]
struct CountingRegistry {
    registered: usize,
}

impl BwPostcutoffAnnealRegistry for CountingRegistry {
    fn register_bw_postcutoff(&mut self, _tuner: &BwPostcutoffTuner) -> bool {
        self.registered += 1;
        true
    }
}

#[derive(Serialize)]
struct TunerReadback {
    trigger: &'static str,
    expected: &'static str,
    current: BwPostcutoffConfig,
    adjustment: calyx_sextant::TunerAdjustment,
    vault_files: Vec<FileReadback>,
    ledger_entries: Vec<calyx_sextant::TunerLedgerEntry>,
    warnings: Vec<calyx_sextant::TunerWarning>,
}

#[derive(Serialize)]
struct FileReadback {
    path: String,
    size: u64,
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    #[test]
    fn safe_recall_windows_never_revert(recall in 0.90_f32..1.0) {
        let mut tuner = BwPostcutoffTuner::new(initial());
        for _ in 0..512 {
            tuner.observe(obs(20_000, recall, initial()));
        }

        let adjustment = tuner.maybe_adjust();

        prop_assert!(adjustment.as_ref().is_none_or(|a| a.kind != TunerAdjustmentKind::Revert));
        prop_assert!(tuner.ledger_entries().is_empty());
    }
}
