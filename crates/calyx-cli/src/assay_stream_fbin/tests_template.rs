use std::fs;
use std::path::Path;

use super::format::VectorFormat;
use super::template;
use super::tests_support::Fixture;
use super::write;

#[test]
fn db_template_streams_without_manifest_args_or_json_sidecars() {
    let fixture = Fixture::new("stream-fbin-db-template", 10, 10, 50);
    let mut args = fixture.args(8);
    args.vector_format = VectorFormat::I8Bin;
    args.emit_artifacts = false;
    assert!(
        args.manifests.is_empty(),
        "gate run must not use manifest args"
    );

    let evidence = write::run(&args).unwrap();

    assert_eq!(evidence.artifact_mode, "db_only");
    assert_eq!(
        evidence.lens_descriptor_source,
        "aster_graph_cf_lens_template"
    );
    let readback = evidence.lens_template_db_readback.unwrap();
    assert!(readback.readback_matches);
    assert_eq!(readback.descriptor_count, 10);
    assert_eq!(readback.lens_names[0], "lens-0");
    assert!(
        evidence.lens_roster[0]
            .manifest
            .starts_with("aster-graph-cf:"),
        "lens roster must cite DB template authority"
    );
    assert!(fixture.out.join("partitioned_rrf_plan_cf").exists());
    assert!(fixture.out.join("partitioned_rrf_timeline_cf").exists());
    assert_eq!(json_count(&fixture.out), 0);
    let temp = fixture
        .out
        .with_file_name(format!(".out.lens-template-{}", std::process::id()));
    assert!(
        !temp.exists(),
        "DB-native streams must not create temporary manifest materializations"
    );
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn lens_template_import_refuses_duplicate_key() {
    let fixture = Fixture::new("stream-fbin-template-duplicate", 10, 10, 50);
    let record = template::record_from_manifests(&fixture.manifest_paths()).unwrap();
    let root = fixture.root.join("duplicate_template_cf");

    template::write(&root, "unit_template", &record).unwrap();
    let error = template::write(&root, "unit_template", &record).unwrap_err();

    assert_eq!(
        error.code,
        "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_DB_EXISTS"
    );
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn direct_lens_template_import_writes_db_row_without_json_sidecars() {
    let root = std::env::temp_dir().join(format!(
        "calyx-stream-fbin-direct-template-{}",
        std::process::id()
    ));
    let raw = direct_template_args(&root);

    template::run_import(&raw).unwrap();

    let (record, readback) = template::read(&root, "unit_direct_template").unwrap();
    assert!(readback.readback_matches);
    assert_eq!(record.descriptors.len(), 13);
    // The roster preserves import order; the first lens is the leading
    // `--algorithmic` entry, imported with the algorithmic runtime contract.
    assert_eq!(record.descriptors[0].name, "gdelt-cameo-event-code");
    assert_eq!(record.descriptors[0].runtime, "algorithmic");
    assert_eq!(record.descriptors[0].dtype, "f32");
    assert_eq!(record.descriptors[0].max_batch, None);
    assert!(
        record
            .descriptors
            .iter()
            .all(|descriptor| descriptor.source_path.starts_with("calyx-db-direct:")),
        "direct roster must not cite filesystem manifests"
    );
    for expected in [
        "gdelt-action-geo",
        "gdelt-actor-country",
        "gdelt-source-host",
        "gdelt-sqldate",
        "gdelt-event-code",
    ] {
        assert!(
            record
                .descriptors
                .iter()
                .any(|descriptor| descriptor.name == expected),
            "direct roster missing {expected}"
        );
    }
    assert_eq!(json_count(&root), 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn manifest_and_direct_lens_template_imports_merge_into_one_roster() {
    // The importer now *allows* mixing file `--manifest` lenses with direct
    // `--algorithmic`/`--tei` lenses: they merge (deduplicated by name) into one
    // roster instead of being refused. This is the current contract — the prior
    // "cannot mix --manifest with direct" refusal was removed.
    // A35 requires >=10 lenses in a template, so seed 10 manifest lenses and mix
    // in one direct algorithmic lens for 11 total.
    let fixture = Fixture::new("stream-fbin-template-mix", 10, 10, 50);
    let root = fixture.root.join("mixed_template_cf");
    let manifests = fixture.manifest_paths();

    let mut raw = vec![
        "--cf-root".to_string(),
        root.display().to_string(),
        "--lens-template-key".to_string(),
        "unit_direct_template_mix".to_string(),
    ];
    for manifest in &manifests {
        raw.push("--manifest".to_string());
        raw.push(manifest.display().to_string());
    }
    // One offline direct algorithmic lens mixed in alongside the manifests.
    raw.push("--algorithmic".to_string());
    raw.push("direct-gdelt-tone".to_string());
    raw.push("gdelt-tone-signal".to_string());
    raw.push("512".to_string());

    template::run_import(&raw).unwrap();

    let (record, readback) = template::read(&root, "unit_direct_template_mix").unwrap();
    assert!(readback.readback_matches);
    // Ten manifest lenses plus the one direct algorithmic lens, merged.
    assert_eq!(record.descriptors.len(), 11);
    let names: Vec<&str> = record
        .descriptors
        .iter()
        .map(|descriptor| descriptor.name.as_str())
        .collect();
    assert!(names.contains(&"lens-0"));
    assert!(names.contains(&"lens-1"));
    assert!(names.contains(&"direct-gdelt-tone"));
    // The direct lens cites the DB; the manifest lenses cite their files.
    let direct = record
        .descriptors
        .iter()
        .find(|descriptor| descriptor.name == "direct-gdelt-tone")
        .expect("direct algorithmic lens present");
    assert!(direct.source_path.starts_with("calyx-db-direct:"));
    let manifest_lens = record
        .descriptors
        .iter()
        .find(|descriptor| descriptor.name == "lens-0")
        .expect("manifest lens present");
    assert!(!manifest_lens.source_path.starts_with("calyx-db-direct:"));
    // The DB-native import path writes no JSON sidecars.
    assert_eq!(json_count(&root), 0);
    let _ = fs::remove_dir_all(fixture.root);
}

fn direct_template_args(root: &Path) -> Vec<String> {
    // Direct import exercised entirely through offline `--algorithmic` lenses.
    // The `--tei` path now attests each lens against a *live* TEI `/info` service
    // and a physical model artifact (`--tei <name> <endpoint> <dim>
    // <physical-model-artifact>`), so it can no longer be driven from an offline
    // unit test; algorithmic lenses fully cover the DB-row/no-JSON-sidecar intent.
    let mut raw = vec![
        "--cf-root".to_string(),
        root.display().to_string(),
        "--lens-template-key".to_string(),
        "unit_direct_template".to_string(),
    ];
    for (name, kind, dim) in [
        ("gdelt-cameo-event-code", "gdelt-cameo", "16"),
        ("gdelt-actor-geo-entity", "gdelt-actor-geo", "512"),
        ("gdelt-source-domain", "gdelt-source-domain", "512"),
        ("gdelt-event-geo", "gdelt-event-geo", "512"),
        ("gdelt-actor-pair", "gdelt-actor-pair", "512"),
        ("gdelt-event-actor", "gdelt-event-actor", "512"),
        ("gdelt-tone-signal", "gdelt-tone-signal", "512"),
        ("gdelt-source-event", "gdelt-source-event", "512"),
        ("gdelt-action-geo", "gdelt-action-geo", "512"),
        ("gdelt-actor-country", "gdelt-actor-country", "512"),
        ("gdelt-source-host", "gdelt-source-host", "512"),
        ("gdelt-sqldate", "gdelt-sqldate", "512"),
        ("gdelt-event-code", "gdelt-event-code", "512"),
    ] {
        raw.push("--algorithmic".to_string());
        raw.push(name.to_string());
        raw.push(kind.to_string());
        raw.push(dim.to_string());
    }
    raw
}

fn json_count(root: &Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count += json_count(&path);
            } else if matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("json" | "jsonl")
            ) {
                count += 1;
            }
        }
    }
    count
}
