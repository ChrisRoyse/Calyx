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
    assert_eq!(record.descriptors.len(), 15);
    assert_eq!(record.descriptors[0].name, "semantic-e5-base-tei");
    assert_eq!(record.descriptors[0].dtype, "float16");
    assert_eq!(record.descriptors[0].max_batch, Some(64));
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
fn direct_lens_template_import_refuses_manifest_mix() {
    let root = std::env::temp_dir().join(format!(
        "calyx-stream-fbin-direct-template-mix-{}",
        std::process::id()
    ));
    let mut raw = vec![
        "--manifest".to_string(),
        "legacy.json".to_string(),
        "--cf-root".to_string(),
        root.display().to_string(),
        "--tei".to_string(),
        "semantic-e5-base-tei".to_string(),
        "http://127.0.0.1:18190/embed".to_string(),
        "768".to_string(),
    ];
    raw.push("--lens-template-key".to_string());
    raw.push("unit_direct_template_mix".to_string());

    let error = template::run_import(&raw).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cannot mix --manifest with direct --tei/--algorithmic")
    );
    let _ = fs::remove_dir_all(root);
}

fn direct_template_args(root: &Path) -> Vec<String> {
    let mut raw = vec![
        "--cf-root".to_string(),
        root.display().to_string(),
        "--lens-template-key".to_string(),
        "unit_direct_template".to_string(),
        "--tei".to_string(),
        "semantic-e5-base-tei".to_string(),
        "http://127.0.0.1:18190/embed".to_string(),
        "768".to_string(),
        "--tei".to_string(),
        "semantic-bge-m3-tei".to_string(),
        "http://127.0.0.1:18188/embed".to_string(),
        "1024".to_string(),
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
