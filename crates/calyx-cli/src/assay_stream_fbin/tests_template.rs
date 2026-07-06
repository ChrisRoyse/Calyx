use std::fs;
use std::path::Path;

use super::format::VectorFormat;
use super::template;
use super::write;

#[path = "tests/support.rs"]
mod support;

use support::Fixture;

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
