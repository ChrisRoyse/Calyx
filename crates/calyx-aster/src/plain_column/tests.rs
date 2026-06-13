use super::*;
use calyx_core::VaultId;

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn new_vault() -> AsterVault<calyx_core::SystemClock> {
    AsterVault::new(vault_id(), b"salt")
}

/// Synthetic 3-row corpus with deliberately sparse columns:
///   user:1 -> {age:30,  city:nyc, name:alice}
///   user:2 -> {age:25,            name:bob}    (no city)
///   user:3 -> {         city:la,  name:carol}  (no age)
fn seed(col: &PlainColumn<'_, calyx_core::SystemClock>) -> Seq {
    col.put(b"user:1", b"name", b"alice").unwrap();
    col.put(b"user:1", b"age", b"30").unwrap();
    col.put(b"user:1", b"city", b"nyc").unwrap();
    col.put(b"user:2", b"name", b"bob").unwrap();
    col.put(b"user:2", b"age", b"25").unwrap();
    col.put(b"user:3", b"name", b"carol").unwrap();
    col.put(b"user:3", b"city", b"la").unwrap().seq
}

fn cells(out: &[WideCell]) -> Vec<(String, String, String)> {
    out.iter()
        .map(|c| {
            (
                String::from_utf8_lossy(&c.row).into_owned(),
                String::from_utf8_lossy(&c.column).into_owned(),
                String::from_utf8_lossy(&c.value).into_owned(),
            )
        })
        .collect()
}

#[test]
fn put_writes_both_physical_keys_into_the_graph_cf() {
    // Source of truth: raw rows in the Graph CF. We never trust put()'s return
    // value alone — we re-read the CF and confirm both keys carry the value.
    let vault = new_vault();
    let col = PlainColumn::new(&vault, "people").unwrap();
    let commit = col.put(b"user:1", b"name", b"alice").unwrap();

    let raw = vault.scan_cf_at(commit.seq, ColumnFamily::Graph).unwrap();
    let cell = raw.iter().find(|(k, _)| k == &commit.cell_key);
    let index = raw.iter().find(|(k, _)| k == &commit.index_key);
    assert!(cell.is_some(), "row-major cell key absent from Graph CF");
    assert!(
        index.is_some(),
        "column-major index key absent from Graph CF"
    );
    assert_eq!(cell.unwrap().1, b"alice");
    assert_eq!(index.unwrap().1, b"alice");
    // The two physical keys must be distinct rows.
    assert_ne!(commit.cell_key, commit.index_key);
}

#[test]
fn scan_column_returns_only_rows_that_carry_it_never_zero_filled() {
    // 2+2=4 discipline: hand-computed expected outputs for each column.
    let vault = new_vault();
    let col = PlainColumn::new(&vault, "people").unwrap();
    let seq = seed(&col);

    // name: present on all three rows, in row order.
    assert_eq!(
        cells(&col.scan_column(seq, b"name", 16).unwrap()),
        vec![
            ("user:1".into(), "name".into(), "alice".into()),
            ("user:2".into(), "name".into(), "bob".into()),
            ("user:3".into(), "name".into(), "carol".into()),
        ]
    );
    // age: user:3 has none -> it is absent, NOT zero-filled.
    assert_eq!(
        cells(&col.scan_column(seq, b"age", 16).unwrap()),
        vec![
            ("user:1".into(), "age".into(), "30".into()),
            ("user:2".into(), "age".into(), "25".into()),
        ]
    );
    // city: user:2 has none -> absent.
    assert_eq!(
        cells(&col.scan_column(seq, b"city", 16).unwrap()),
        vec![
            ("user:1".into(), "city".into(), "nyc".into()),
            ("user:3".into(), "city".into(), "la".into()),
        ]
    );
}

#[test]
fn sparse_absence_has_no_physical_index_key() {
    // Prove the absence is structural: there is NO age index key for user:3.
    let vault = new_vault();
    let col = PlainColumn::new(&vault, "people").unwrap();
    let seq = seed(&col);

    // get() of an absent cell is explicit None.
    assert_eq!(col.get(seq, b"user:3", b"age").unwrap(), None);
    assert_eq!(col.get(seq, b"user:2", b"city").unwrap(), None);

    // Independent CF read: the would-be index key simply is not present.
    let ks = ColumnKeyspace::new("people").unwrap();
    let missing = ks.index_key(b"age", b"user:3").unwrap();
    let raw = vault.scan_cf_at(seq, ColumnFamily::Graph).unwrap();
    assert!(
        raw.iter().all(|(k, _)| k != &missing),
        "absent cell must have no physical key"
    );
}

#[test]
fn scan_row_returns_columns_in_lexicographic_order() {
    let vault = new_vault();
    let col = PlainColumn::new(&vault, "people").unwrap();
    let seq = seed(&col);

    // user:1 has age, city, name -> sorted by column name.
    assert_eq!(
        cells(&col.scan_row(seq, b"user:1", 16).unwrap()),
        vec![
            ("user:1".into(), "age".into(), "30".into()),
            ("user:1".into(), "city".into(), "nyc".into()),
            ("user:1".into(), "name".into(), "alice".into()),
        ]
    );
    // user:2 lacks city.
    assert_eq!(
        cells(&col.scan_row(seq, b"user:2", 16).unwrap()),
        vec![
            ("user:2".into(), "age".into(), "25".into()),
            ("user:2".into(), "name".into(), "bob".into()),
        ]
    );
}

#[test]
fn scan_row_columns_is_half_open() {
    let vault = new_vault();
    let col = PlainColumn::new(&vault, "people").unwrap();
    let seq = seed(&col);

    // [age, name) over user:1 -> age, city (name excluded).
    assert_eq!(
        cells(
            &col.scan_row_columns(seq, b"user:1", b"age", b"name", 16)
                .unwrap()
        ),
        vec![
            ("user:1".into(), "age".into(), "30".into()),
            ("user:1".into(), "city".into(), "nyc".into()),
        ]
    );
}

#[test]
fn overwrite_updates_both_keys_without_duplicating_the_cell() {
    let vault = new_vault();
    let col = PlainColumn::new(&vault, "people").unwrap();
    col.put(b"user:1", b"name", b"alice").unwrap();
    let after = col.put(b"user:1", b"name", b"ALICE2").unwrap();

    // get reflects the new value.
    assert_eq!(
        col.get(after.seq, b"user:1", b"name").unwrap(),
        Some(b"ALICE2".to_vec())
    );
    // exactly one cell on the row scan and one on the column scan (no dup).
    let row = col.scan_row(after.seq, b"user:1", 16).unwrap();
    assert_eq!(row.len(), 1);
    assert_eq!(row[0].value, b"ALICE2");
    let column = col.scan_column(after.seq, b"name", 16).unwrap();
    assert_eq!(column.len(), 1);
    assert_eq!(column[0].value, b"ALICE2");
}

#[test]
fn empty_scans_return_empty_not_error() {
    let vault = new_vault();
    let col = PlainColumn::new(&vault, "people").unwrap();
    let seq = seed(&col);
    assert!(col.scan_column(seq, b"nonexistent", 16).unwrap().is_empty());
    assert!(col.scan_row(seq, b"user:404", 16).unwrap().is_empty());
}

#[test]
fn scan_limit_is_bounded_by_construction() {
    let vault = new_vault();
    let col = PlainColumn::new(&vault, "people").unwrap();
    let seq = seed(&col);
    // name has exactly 3 rows: limit 3 is fine, limit 2 fails closed.
    assert_eq!(col.scan_column(seq, b"name", 3).unwrap().len(), 3);
    let err = col.scan_column(seq, b"name", 2).unwrap_err();
    assert_eq!(err.code, "CALYX_WIDECOLUMN_SCAN_LIMIT");
}

#[test]
fn invalid_inputs_fail_closed_with_codes() {
    let vault = new_vault();
    let col = PlainColumn::new(&vault, "people").unwrap();
    assert_eq!(
        col.get(vault.latest_seq(), b"user:1", b"")
            .unwrap_err()
            .code,
        "CALYX_WIDECOLUMN_INVALID_KEY"
    );
    assert_eq!(
        col.put(b"", b"name", b"x").unwrap_err().code,
        "CALYX_WIDECOLUMN_INVALID_KEY"
    );
    let huge = vec![b'x'; (1 << 20) + 1];
    assert_eq!(
        col.put(b"user:1", b"name", &huge).unwrap_err().code,
        "CALYX_WIDECOLUMN_INVALID_KEY"
    );
}

#[test]
fn distinct_collections_are_isolated() {
    let vault = new_vault();
    let people = PlainColumn::new(&vault, "people").unwrap();
    let places = PlainColumn::new(&vault, "places").unwrap();
    people.put(b"r1", b"name", b"alice").unwrap();
    let seq = places.put(b"r1", b"name", b"nyc").unwrap().seq;
    // Each collection sees only its own cell.
    assert_eq!(
        people.get(seq, b"r1", b"name").unwrap(),
        Some(b"alice".to_vec())
    );
    assert_eq!(
        places.get(seq, b"r1", b"name").unwrap(),
        Some(b"nyc".to_vec())
    );
    assert_eq!(people.scan_column(seq, b"name", 16).unwrap().len(), 1);
    assert_eq!(places.scan_column(seq, b"name", 16).unwrap().len(), 1);
}

#[test]
fn binary_row_and_column_keys_round_trip() {
    // Wide-column keys are byte strings, not just UTF-8.
    let vault = new_vault();
    let col = PlainColumn::new(&vault, "bin").unwrap();
    let row = &[0x00, 0xff, 0x10, 0x77][..];
    let column = &[0xde, 0xad, 0xbe, 0xef][..];
    let commit = col.put(row, column, b"v").unwrap();
    assert_eq!(
        col.get(commit.seq, row, column).unwrap(),
        Some(b"v".to_vec())
    );
    let scanned = col.scan_column(commit.seq, column, 16).unwrap();
    assert_eq!(scanned.len(), 1);
    assert_eq!(scanned[0].row, row);
}
