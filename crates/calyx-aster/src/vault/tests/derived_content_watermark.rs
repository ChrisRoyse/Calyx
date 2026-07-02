//! Issue #1100: the derived-content watermark must survive checkpoint +
//! cold reopen so a separate search process can distinguish content-neutral
//! ledger appends from commits that changed derived-search inputs.

use super::*;
use crate::manifest::ManifestStore;
use calyx_core::FixedClock;
use calyx_ledger::ActorId;

fn neutral_ledger_append(vault: &AsterVault) -> u64 {
    vault
        .append_ledger_entry(
            EntryKind::Assay,
            SubjectId::Query(vec![0x11; 16]),
            br#"{"tag":"issue1100-replay-marker"}"#.to_vec(),
            ActorId::Service("issue1100-test".to_string()),
        )
        .expect("append content-neutral ledger entry");
    vault.latest_seq()
}

#[test]
fn watermark_survives_checkpoint_and_cold_reopen() {
    let dir = test_dir("derived-watermark");
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
            .expect("open durable");
    let cx = sample_constellation(&AsterVault::with_clock(
        vault_id(),
        b"salt".to_vec(),
        FixedClock::new(123),
    ));
    vault.put(cx).expect("durable put");
    vault.flush().expect("flush content");
    let content_seq = vault.latest_seq();
    assert_eq!(vault.derived_content_seq(), content_seq);

    // Content-neutral appends (idempotent replay ledger rows): the raw seq
    // advances, the watermark must not.
    let after_first = neutral_ledger_append(&vault);
    let after_second = neutral_ledger_append(&vault);
    assert!(after_second > after_first && after_first > content_seq);
    assert_eq!(vault.derived_content_seq(), content_seq);
    vault.flush().expect("flush neutral appends");

    // Physical source of truth: the durable MANIFEST records the watermark.
    let manifest = ManifestStore::open(&dir).load_current().expect("manifest");
    assert_eq!(manifest.durable_seq, after_second);
    assert_eq!(manifest.derived_content_seq, Some(content_seq));
    assert_eq!(manifest.effective_derived_content_seq(), content_seq);
    drop(vault);

    // Cold reopen (the separate-search-process case).
    let reopened = AsterVault::open(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
        .expect("cold open");
    assert_eq!(reopened.latest_seq(), after_second);
    assert_eq!(reopened.derived_content_seq(), content_seq);
    let pin = reopened.pin_reader(crate::mvcc::Freshness::FreshDerived, 60_000);
    assert_eq!(pin.seq(), after_second);
    assert_eq!(pin.derived_content_seq(), content_seq);
    reopened.release_reader(pin.lease().id());
    cleanup(dir);
}

#[test]
fn uncheckpointed_neutral_appends_replay_without_advancing_watermark() {
    let dir = test_dir("derived-watermark-wal");
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
            .expect("open durable");
    let cx = sample_constellation(&AsterVault::with_clock(
        vault_id(),
        b"salt".to_vec(),
        FixedClock::new(123),
    ));
    vault.put(cx).expect("durable put");
    vault.flush().expect("flush content");
    let content_seq = vault.latest_seq();
    // Neutral append left ONLY in the WAL (no flush): reopen must re-derive
    // its neutrality from the replayed batch's CFs.
    let tip = neutral_ledger_append(&vault);
    drop(vault);

    let reopened = AsterVault::open(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
        .expect("cold open with WAL tail");
    assert_eq!(reopened.latest_seq(), tip);
    assert_eq!(
        reopened.derived_content_seq(),
        content_seq,
        "WAL-replayed ledger append must stay content-neutral"
    );
    cleanup(dir);
}

#[test]
fn neutral_manifest_write_never_regresses_a_foreign_content_watermark() {
    let dir = test_dir("derived-watermark-foreign");
    let template = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));

    // Long-lived local handle: checkpoints its own content at some seq.
    let local =
        AsterVault::new_durable(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
            .expect("open local handle");
    local
        .put(sample_constellation(&template))
        .expect("local content");
    local.flush().expect("flush local content");
    let local_content_seq = local.latest_seq();

    // Foreign writer: a second handle on the same vault directory ingests
    // content and checkpoints; the manifest now vouches for foreign_seq.
    let foreign = AsterVault::open(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
        .expect("open foreign handle");
    let input = b"foreign-input";
    let mut cx = sample_constellation(&template);
    cx.cx_id = foreign.cx_id_for_input(input, 7);
    cx.input_ref.hash = [0_u8; 32];
    cx.input_ref.hash[..input.len()].copy_from_slice(input);
    cx.input_ref.pointer = Some("synthetic://foreign-input".to_string());
    foreign.put(cx).expect("foreign content");
    foreign.flush().expect("flush foreign content");
    let foreign_seq = ManifestStore::open(&dir)
        .load_current()
        .expect("manifest after foreign content")
        .effective_derived_content_seq();
    assert!(foreign_seq > local_content_seq);

    // Content-neutral write + flush from the local handle. Its atomic never
    // saw the foreign checkpoint; the rewritten manifest must not regress the
    // watermark below foreign_seq, or a stale index would pass freshness.
    neutral_ledger_append(&local);
    local.flush().expect("flush neutral append");
    let manifest = ManifestStore::open(&dir)
        .load_current()
        .expect("manifest after neutral write");
    assert!(
        manifest.effective_derived_content_seq() >= foreign_seq,
        "neutral write regressed the derived-content watermark: manifest vouches for {} but foreign content is at {foreign_seq}",
        manifest.effective_derived_content_seq()
    );
    cleanup(dir);
}

#[test]
fn legacy_manifest_without_watermark_fails_closed_to_durable_seq() {
    let dir = test_dir("derived-watermark-legacy");
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
            .expect("open durable");
    let cx = sample_constellation(&AsterVault::with_clock(
        vault_id(),
        b"salt".to_vec(),
        FixedClock::new(123),
    ));
    vault.put(cx).expect("durable put");
    neutral_ledger_append(&vault);
    vault.flush().expect("flush all");
    let tip = vault.latest_seq();
    drop(vault);

    // Simulate a pre-#1100 manifest: strip the recorded watermark.
    let store = ManifestStore::open(&dir);
    let mut manifest = store.load_current().expect("manifest");
    manifest.derived_content_seq = None;
    manifest.manifest_seq += 1;
    store
        .write_current(&manifest)
        .expect("write legacy manifest");

    let reopened = AsterVault::open(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
        .expect("cold open legacy");
    assert_eq!(
        reopened.derived_content_seq(),
        tip,
        "legacy manifests must fail closed to durable_seq (exact-equality semantics)"
    );
    cleanup(dir);
}
