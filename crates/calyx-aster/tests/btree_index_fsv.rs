//! Full-State-Verification harness for the PH54 T01 btree index key encoding.
//!
//! Source of truth: the raw key bytes the public encoder emits. Each case prints
//! `input -> hand-computed expected -> actual` and asserts byte equality, then
//! reads the bytes back through `decode_index_key` to prove the round-trip. Run:
//!
//! ```text
//! cargo test -p calyx-aster --test btree_index_fsv -- --nocapture
//! ```

use calyx_aster::collection::FieldType;
use calyx_aster::index::{BtreeIndex, IndexId, IndexKind, IndexSpec, SecondaryIndex};
use calyx_aster::layers::{RecordKey, RecordValue};

const COLLECTION_ID: u64 = 0x0102_0304_0506_0708;
const INDEX_ID: u32 = 0x0A0B_0C0D;

fn index(field_type: FieldType) -> BtreeIndex {
    BtreeIndex::new(
        COLLECTION_ID,
        IndexSpec::new(
            IndexId::new(INDEX_ID),
            "ix",
            IndexKind::Btree,
            "f",
            field_type,
        ),
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

/// The 13-byte fixed prefix every key carries: 0x10 | collection | index.
const PREFIX_HEX: &str = "100102030405060708 0a0b0c0d";

fn field_component(key: &[u8]) -> &[u8] {
    // 13-byte prefix; trailing 8 bytes are the `from_u64` primary key.
    &key[13..key.len() - 8]
}

#[test]
fn fsv_synthetic_known_io_and_edges() {
    println!("\n=== PH54 T01 btree index FSV — prefix = {PREFIX_HEX} ===");

    // --- Happy path: I64 golden bytes (2+2=4 discipline) ---------------------
    // -1 as u64 = 0xffff_ffff_ffff_ffff; XOR 0x8000... = 0x7fff_ffff_ffff_ffff.
    let idx_i64 = index(FieldType::I64);
    let key = idx_i64
        .encode_index_key(&RecordValue::I64(-1), &RecordKey::from_u64(0))
        .unwrap();
    let expected = "7fffffffffffffff";
    let actual = hex(field_component(&key));
    println!(
        "I64(-1)  expected field={expected}  actual field={actual}  full={}",
        hex(&key)
    );
    assert_eq!(actual, expected, "I64(-1) golden field bytes");
    assert_eq!(
        idx_i64.decode_index_key(&key).unwrap().0,
        RecordValue::I64(-1)
    );

    // --- Monotonic byte order proves correct range-scan ordering -------------
    let ordered = [-5_i64, -1, 0, 3, i64::MAX];
    let mut prev: Option<Vec<u8>> = None;
    println!("-- I64 ordering (byte order must equal numeric order) --");
    for v in ordered {
        let k = idx_i64
            .encode_index_key(&RecordValue::I64(v), &RecordKey::from_u64(0))
            .unwrap();
        println!("  I64({v:>20}) field={}", hex(field_component(&k)));
        if let Some(p) = &prev {
            assert!(p < &k, "I64 ordering broken at {v}");
        }
        prev = Some(k);
    }

    // --- F64 total-order: -1.0 < 0.0 < 1.0 -----------------------------------
    let idx_f64 = index(FieldType::F64);
    for v in [-1.0_f64, 0.0, 1.0] {
        let k = idx_f64
            .encode_index_key(&RecordValue::F64(v), &RecordKey::from_u64(0))
            .unwrap();
        println!("F64({v:>5}) field={}", hex(field_component(&k)));
    }
    let fneg = idx_f64
        .encode_index_key(&RecordValue::F64(-1.0), &RecordKey::from_u64(0))
        .unwrap();
    let fzero = idx_f64
        .encode_index_key(&RecordValue::F64(0.0), &RecordKey::from_u64(0))
        .unwrap();
    let fpos = idx_f64
        .encode_index_key(&RecordValue::F64(1.0), &RecordKey::from_u64(0))
        .unwrap();
    assert!(fneg < fzero && fzero < fpos, "F64 total order broken");

    // --- Edge 1: empty Text -> bare 2-byte terminator 0x00 0x01 --------------
    let idx_text = index(FieldType::Text);
    let empty = idx_text
        .encode_index_key(&RecordValue::Text(String::new()), &RecordKey::from_u64(0))
        .unwrap();
    println!(
        "Edge[empty Text] field={} (expected 0001)",
        hex(field_component(&empty))
    );
    assert_eq!(hex(field_component(&empty)), "0001");

    // --- Edge 2: Text truncation at 64 bytes ---------------------------------
    let long = "x".repeat(70);
    let ktrunc = idx_text
        .encode_index_key(&RecordValue::Text(long), &RecordKey::from_u64(0))
        .unwrap();
    let decoded = idx_text.decode_index_key(&ktrunc).unwrap().0;
    println!(
        "Edge[70-byte Text] decoded len = {}",
        match &decoded {
            RecordValue::Text(s) => s.len(),
            _ => 0,
        }
    );
    assert_eq!(decoded, RecordValue::Text("x".repeat(64)));

    // --- Edge 3: embedded NUL escapes to 0x00 0xff and round-trips -----------
    let nul = idx_text
        .encode_index_key(
            &RecordValue::Text("a\u{0}b".into()),
            &RecordKey::from_u64(9),
        )
        .unwrap();
    println!(
        "Edge[Text a\\0b] field={} (00 escaped to 00ff)",
        hex(field_component(&nul))
    );
    assert_eq!(hex(field_component(&nul)), "6100ff620001"); // 'a' 00->00ff 'b' term
    let (dv, dpk) = idx_text.decode_index_key(&nul).unwrap();
    assert_eq!(dv, RecordValue::Text("a\u{0}b".into()));
    assert_eq!(dpk, RecordKey::from_u64(9));

    // --- Fail-closed: wrong type, NULL, short key ----------------------------
    println!("-- fail-closed paths --");
    let e1 = idx_i64
        .encode_index_key(&RecordValue::Text("x".into()), &RecordKey::from_u64(0))
        .unwrap_err();
    println!("  type mismatch -> {}", e1.code);
    assert_eq!(e1.code, "CALYX_INVALID_ARGUMENT");
    let e2 = idx_i64
        .encode_index_key(&RecordValue::Null, &RecordKey::from_u64(0))
        .unwrap_err();
    println!("  NULL value    -> {}", e2.code);
    assert_eq!(e2.code, "CALYX_INVALID_ARGUMENT");
    let e3 = idx_i64.decode_index_key(&[0x10, 0x00, 0x00]).unwrap_err();
    println!("  3-byte key    -> {}", e3.code);
    assert_eq!(e3.code, "CALYX_ASTER_CORRUPT_SHARD");

    println!("=== FSV PASS: bytes match hand-computed expectations ===\n");
}
