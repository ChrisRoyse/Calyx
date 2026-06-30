use super::*;

#[test]
fn read_only_open_skips_ledger_hook_and_rejects_writes_before_wal_append() {
    let dir = test_dir("read-only-open");
    let writer =
        AsterVault::new_durable(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
            .expect("open durable writer");
    let cx = sample_constellation(&AsterVault::with_clock(
        vault_id(),
        b"salt".to_vec(),
        FixedClock::new(123),
    ));
    let id = cx.cx_id;
    writer.put(cx).expect("seed durable row");
    writer.flush().expect("flush durable row");
    let stored = writer.get(id, writer.snapshot()).expect("writer readback");
    let before_seq = writer.snapshot();
    let before_wal_bytes = wal_bytes(&dir);
    drop(writer);

    let reader = AsterVault::open(
        &dir,
        vault_id(),
        b"salt".to_vec(),
        VaultOptions {
            restore_mvcc_rows: false,
            restore_ledger_hook: false,
            read_only: true,
            ..VaultOptions::default()
        },
    )
    .expect("open read-only latest vault");

    assert!(!reader.has_real_ledger_hook());
    assert_eq!(reader.snapshot(), before_seq);
    assert_eq!(reader.get(id, reader.snapshot()).unwrap(), stored);

    let mut attempted = sample_constellation(&AsterVault::with_clock(
        vault_id(),
        b"salt".to_vec(),
        FixedClock::new(456),
    ));
    let input = b"read-only-write-attempt";
    attempted.cx_id = reader.cx_id_for_input(input, attempted.panel_version);
    attempted.input_ref.hash = [0; 32];
    attempted.input_ref.hash[..input.len()].copy_from_slice(input);
    attempted.input_ref.pointer = Some("synthetic://read-only-write-attempt".to_string());
    let attempted_key = base_key(attempted.cx_id);
    let err = reader
        .put(attempted.clone())
        .expect_err("read-only handle rejects mutation");

    assert_eq!(err.code, "CALYX_VAULT_READ_ONLY");
    assert_eq!(reader.snapshot(), before_seq);
    assert_eq!(wal_bytes(&dir), before_wal_bytes);
    assert_eq!(
        reader
            .read_cf_at(reader.snapshot(), ColumnFamily::Base, &attempted_key)
            .unwrap(),
        None
    );
    drop(reader);

    let reopened = AsterVault::open(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
        .expect("cold reopen after rejected read-only write");
    assert_eq!(reopened.snapshot(), before_seq);
    assert_eq!(wal_bytes(&dir), before_wal_bytes);
    assert_eq!(
        reopened
            .read_cf_at(reopened.snapshot(), ColumnFamily::Base, &attempted_key)
            .unwrap(),
        None
    );
    cleanup(dir);
}
