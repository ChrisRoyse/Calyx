use super::*;

/// Issue #1985 end-to-end: a real multi-chunk batch ingest accumulates
/// per-commit flush SSTs, and the ingest lifecycle must compact them so no
/// active CF is left with an unbounded file count. Pre-fix, `calyx ingest`
/// never compacted and a large build reached millions of SSTs.
///
/// The default debt trigger is 12 (see command/compaction.rs
/// `DEFAULT_TRIGGER_FILES`); this asserts the physical on-disk file count for
/// the high-volume point-read CFs is bounded by it after ingest, and that
/// every ingested constellation reads back.
#[test]
fn batch_ingest_bounds_per_cf_flush_sst_files() {
    const DEFAULT_TRIGGER_FILES: usize = 12;
    let (root, resolved) = test_vault_with_registered_dense_lens("compaction-lifecycle");
    let jsonl = resolved.path.join("compaction.jsonl");

    // Enough distinct rows to span more than one measure window (default 128),
    // so a mid-stream chunk-boundary compaction runs in addition to the final
    // barrier, and enough per-commit flushes accrue to cross the trigger.
    let row_count = 160usize;
    let texts: Vec<String> = (0..row_count)
        .map(|i| format!("compaction lifecycle unique payload row {i:04}"))
        .collect();
    let body = texts
        .iter()
        .map(|text| batch_line(text))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&jsonl, format!("{body}\n")).unwrap();

    let summary = ingest_batch_streaming(&resolved, &jsonl).unwrap();
    assert_eq!(summary.row_count, row_count);

    // Physical proof: the accumulating point-read CFs are bounded by the
    // trigger after ingest (was thousands of files before the fix).
    for cf in [ColumnFamily::Base, ColumnFamily::slot(SlotId::new(0))] {
        let files = cf_file_count(&resolved.path, cf);
        assert!(
            files <= DEFAULT_TRIGGER_FILES,
            "CF {} left with {files} SST files, above trigger {DEFAULT_TRIGGER_FILES}",
            cf.name()
        );
    }

    // Every constellation survives compaction + input reclaim.
    let vault = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();
    let snapshot = vault.snapshot();
    for text in &texts {
        let cx_id = vault.cx_id_for_input(text.as_bytes(), state.panel.version);
        let cx = vault.get(cx_id, snapshot).unwrap();
        assert_eq!(
            cx.cx_id, cx_id,
            "ingested constellation lost after compaction"
        );
    }
    drop(vault);
    fs::remove_dir_all(root).ok();
}

/// A set-but-garbage compaction trigger override fails resolution closed
/// before any authoritative work (issue #1985 §1). Exercised through the pure
/// resolver rather than by mutating process-global environment, which would
/// race every parallel test that runs an ingest (the ingest read path is not
/// serialized on any env lock, by design — config is read once up front).
#[test]
fn batch_ingest_rejects_garbage_compaction_trigger_env() {
    let key = "CALYX_INGEST_COMPACTION_TRIGGER_FILES";
    for garbage in ["not-a-number", "0", "-3", ""] {
        let error = super::super::command::compaction::resolve_ingest_compaction_config_from(
            Some(garbage.to_string()),
            None,
            None,
        )
        .expect_err("garbage trigger must fail resolution closed");
        assert!(
            error.message().contains(key),
            "unexpected error for {garbage:?}: {}",
            error.message()
        );
    }
    // Unset overrides resolve to validated defaults.
    let config =
        super::super::command::compaction::resolve_ingest_compaction_config_from(None, None, None)
            .expect("defaults must resolve");
    assert!(config.trigger_files >= 1);
}
