use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Modality, QuantPolicy};
use calyx_registry::LensForgeManifest;
use serde_json::Value;

use super::args::Args;
use super::export_fbin;

#[test]
fn export_fbin_writes_headers_plan_and_readback_report() {
    let fixture = Fixture::new("export-fbin-happy", 10, 6);
    let args = fixture.args(2);

    let evidence = export_fbin(&args).unwrap();

    assert_eq!(evidence.rows, 6);
    assert_eq!(evidence.query_count, 2);
    assert_eq!(evidence.lens_roster.len(), 10);
    assert_fbin_header(&fixture.out.join("fbin/slot_00_lens-0_corpus.fbin"), 3, 6);
    assert_fbin_header(&fixture.out.join("fbin/slot_00_lens-0_queries.fbin"), 3, 2);
    let plan: Value =
        serde_json::from_slice(&fs::read(fixture.out.join("partitioned_rrf_plan.json")).unwrap())
            .unwrap();
    assert_eq!(plan["slots"].as_array().unwrap().len(), 10);
    assert_eq!(
        plan["timeline"].as_str().unwrap(),
        fixture.out.join("timeline.jsonl").display().to_string()
    );
    assert_eq!(plan["temporal_counts_toward_a35"], false);
    assert_eq!(plan["slots"][0]["name"], "lens-0");
    let bits = plan["slots"][0]["bits_about"].as_f64().unwrap();
    assert!((bits - 0.2).abs() < 0.00001);
    let timeline = fs::read_to_string(fixture.out.join("timeline.jsonl")).unwrap();
    let timeline_rows = timeline
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(timeline_rows.len(), 6);
    assert_eq!(
        timeline_rows[0]["source_event_time_secs"],
        1_704_153_600_i64
    );
    assert_eq!(timeline_rows[0]["query_row"], true);
    assert_eq!(timeline_rows[2]["query_row"], false);
    assert!(fixture.out.join("export_report.json").is_file());
    let report: Value =
        serde_json::from_slice(&fs::read(fixture.out.join("export_report.json")).unwrap()).unwrap();
    assert_eq!(report["out_dir"], fixture.out.display().to_string());
    assert_eq!(
        report["timeline_path"].as_str().unwrap(),
        fixture.out.join("timeline.jsonl").display().to_string()
    );
    assert_eq!(report["temporal"]["active_rows"], 6);
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn export_fbin_preserves_corpus_build_lens_order() {
    let names = vec![
        "zulu-lens",
        "alpha-lens",
        "mercury-lens",
        "bravo-lens",
        "theta-lens",
        "charlie-lens",
        "omega-lens",
        "delta-lens",
        "kappa-lens",
        "echo-lens",
    ];
    let fixture = Fixture::with_names("export-fbin-corpus-order", &names, 10, 4);
    let args = fixture.args(2);

    let evidence = export_fbin(&args).unwrap();

    let evidence_names = evidence
        .lens_roster
        .iter()
        .map(|lens| lens.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(evidence_names, names);
    let plan: Value =
        serde_json::from_slice(&fs::read(fixture.out.join("partitioned_rrf_plan.json")).unwrap())
            .unwrap();
    let plan_names = plan["slots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|slot| slot["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(plan_names, names);
    assert!(
        fixture
            .out
            .join("fbin/slot_00_zulu-lens_corpus.fbin")
            .is_file()
    );
    assert!(
        fixture
            .out
            .join("fbin/slot_01_alpha-lens_corpus.fbin")
            .is_file()
    );
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn export_fbin_rejects_query_count_above_rows() {
    let fixture = Fixture::new("export-fbin-query-too-large", 10, 3);
    let args = fixture.args(4);

    let error = export_fbin(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_FBIN_EXPORT_QUERY_TOO_LARGE");
    assert!(!fixture.out.exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn export_fbin_rejects_panel_below_a35_floor() {
    let fixture = Fixture::new("export-fbin-too-small", 3, 6);
    let args = fixture.args(2);

    let error = export_fbin(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_FBIN_EXPORT_PANEL_TOO_SMALL");
    assert!(!fixture.out.exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn export_fbin_rejects_inconsistent_vector_dimensions() {
    let fixture = Fixture::new("export-fbin-bad-dim", 10, 6);
    let mut lines = fs::read_to_string(fixture.corpus.join("vectors.jsonl")).unwrap();
    lines.push_str(
        &serde_json::json!({
            "id": "bad-row",
            "lenses": {
                "lens-0": [1.0, 2.0],
                "lens-1": [1.0, 2.0, 3.0],
                "lens-2": [1.0, 2.0, 3.0],
                "lens-3": [1.0, 2.0, 3.0],
                "lens-4": [1.0, 2.0, 3.0],
                "lens-5": [1.0, 2.0, 3.0],
                "lens-6": [1.0, 2.0, 3.0],
                "lens-7": [1.0, 2.0, 3.0],
                "lens-8": [1.0, 2.0, 3.0],
                "lens-9": [1.0, 2.0, 3.0]
            }
        })
        .to_string(),
    );
    lines.push('\n');
    fs::write(fixture.corpus.join("vectors.jsonl"), lines).unwrap();
    let args = fixture.args(2);

    let error = export_fbin(&args).unwrap_err();

    assert_eq!(
        error.code(),
        "CALYX_FSV_ASSAY_FBIN_EXPORT_LENS_SET_MISMATCH"
    );
    assert!(!fixture.out.exists());
    let _ = fs::remove_dir_all(fixture.root);
}

struct Fixture {
    root: PathBuf,
    corpus: PathBuf,
    out: PathBuf,
    bits: PathBuf,
}

impl Fixture {
    fn new(name: &str, admitted_lenses: usize, rows: usize) -> Self {
        let names = (0..10).map(|idx| format!("lens-{idx}")).collect::<Vec<_>>();
        Self::with_names(name, &names, admitted_lenses, rows)
    }

    fn with_names(
        name: &str,
        names: &[impl AsRef<str>],
        admitted_lenses: usize,
        rows: usize,
    ) -> Self {
        let root = temp_root(name);
        let corpus = root.join("corpus");
        let manifests = root.join("manifests");
        let out = root.join("out");
        fs::create_dir_all(&corpus).unwrap();
        fs::create_dir_all(&manifests).unwrap();
        let names = names
            .iter()
            .map(|name| name.as_ref().to_string())
            .collect::<Vec<_>>();
        let manifest_paths = write_manifests(&manifests, &names);
        write_vectors(&corpus.join("vectors.jsonl"), &names, rows);
        write_build_report(
            &corpus.join("corpus_build_report.json"),
            &names,
            &manifest_paths,
        );
        let bits = root.join("assay_abundance.json");
        write_bits(&bits, &names, admitted_lenses);
        Self {
            root,
            corpus,
            out,
            bits,
        }
    }

    fn args(&self, query_count: usize) -> Args {
        Args {
            corpus_dir: self.corpus.clone(),
            out_dir: self.out.clone(),
            bits_report: self.bits.clone(),
            query_count,
            min_bits: 0.05,
        }
    }
}

fn write_vectors(path: &Path, lenses: &[String], rows: usize) {
    let mut lines = String::new();
    for row in 0..rows {
        let lens_map = lenses
            .iter()
            .enumerate()
            .map(|(idx, name)| {
                (
                    name.clone(),
                    serde_json::json!([row as f32 + 0.1, idx as f32 + 0.2, 1.0]),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        lines.push_str(
            &serde_json::json!({
                "id": format!("row-{row}"),
                "source_event_time_secs": 1_704_153_600_i64 + row as i64,
                "source_event_time_raw": format!("{}", 1_704_153_600_i64 + row as i64),
                "temporal_lane_state": "active",
                "source_sequence": "jsonl_line",
                "source_sequence_index": row,
                "lenses": lens_map
            })
            .to_string(),
        );
        lines.push('\n');
    }
    fs::write(path, lines).unwrap();
}

fn write_build_report(path: &Path, names: &[String], manifests: &[PathBuf]) {
    let lenses = manifests
        .iter()
        .zip(names)
        .map(|(manifest, name)| {
            serde_json::json!({
                "name": name,
                "manifest": manifest
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        path,
        serde_json::to_vec_pretty(&serde_json::json!({ "lenses": lenses })).unwrap(),
    )
    .unwrap();
}

fn write_bits(path: &Path, lenses: &[String], admitted: usize) {
    let lenses = lenses
        .iter()
        .enumerate()
        .map(|(idx, name)| {
            serde_json::json!({
                "name": name,
                "bits_about": 0.2,
                "admitted": idx < admitted
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        path,
        serde_json::to_vec_pretty(&serde_json::json!({ "lenses": lenses })).unwrap(),
    )
    .unwrap();
}

fn write_manifests(root: &Path, names: &[String]) -> Vec<PathBuf> {
    names
        .iter()
        .map(|name| {
            let path = root.join(format!("{name}.json"));
            let manifest = LensForgeManifest {
                name: name.clone(),
                modality: Modality::Text,
                runtime: "algorithmic:one-hot:3".to_string(),
                dim: 3,
                dtype: "f32".to_string(),
                weights_sha256: String::new(),
                artifact_set_sha256: None,
                files: Vec::new(),
                pooling: "algorithmic".to_string(),
                norm: "none".to_string(),
                source_hf_id: format!("calyx/{name}"),
                endpoint: None,
                license: Some("apache-2.0".to_string()),
                non_commercial: false,
                quant_default: QuantPolicy::turboquant_default(),
                truncate_dim: None,
                recall_delta: calyx_registry::spec::default_recall_delta(),
                max_batch: None,
            };
            fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
            path
        })
        .collect()
}

fn assert_fbin_header(path: &Path, dim: u32, count: u64) {
    let bytes = fs::read(path).unwrap();
    assert_eq!(&bytes[0..8], b"CLXVEC01");
    assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), dim);
    assert_eq!(u64::from_le_bytes(bytes[12..20].try_into().unwrap()), count);
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}
