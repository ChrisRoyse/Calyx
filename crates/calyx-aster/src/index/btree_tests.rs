//! Synthetic, deterministic FSV for the btree index key encoding (PH54 T01).

use super::*;
use proptest::prelude::*;

const COLLECTION_ID: u64 = 0x0102_0304_0506_0708;
const INDEX_ID: u32 = 0x0A0B_0C0D;

fn index(field_type: FieldType) -> BtreeIndex {
    BtreeIndex::new(
        COLLECTION_ID,
        IndexSpec::new(
            super::super::IndexId::new(INDEX_ID),
            "ix",
            IndexKind::Btree,
            "f",
            field_type,
        ),
    )
}

fn pk(value: u64) -> RecordKey {
    RecordKey::from_u64(value)
}

/// Returns the `field_val_encoded` slice of a key (after the 13-byte prefix,
/// before the 8-byte `from_u64` primary key).
fn field_component(key: &[u8]) -> &[u8] {
    &key[PREFIX_BYTES..key.len() - 8]
}

#[test]
fn i64_negative_one_golden_bytes_and_prefix() {
    let idx = index(FieldType::I64);
    let key = idx.encode_index_key(&RecordValue::I64(-1), &pk(0)).unwrap();
    // Prefix: 0x10 | collection_id | index_id.
    assert_eq!(key[0], DISC_BTREE_INDEX);
    assert_eq!(&key[1..9], &COLLECTION_ID.to_be_bytes());
    assert_eq!(&key[9..13], &INDEX_ID.to_be_bytes());
    // Sign-flip of -1 is 0x7fff_ffff_ffff_ffff.
    assert_eq!(
        field_component(&key),
        &0x7fff_ffff_ffff_ffffu64.to_be_bytes()
    );
    // I64(0) sign-flips to 0x8000... which is byte-greater than -1's encoding.
    let key_zero = idx.encode_index_key(&RecordValue::I64(0), &pk(0)).unwrap();
    assert!(key_zero > key);
}

#[test]
fn i64_byte_order_equals_numeric_order() {
    let idx = index(FieldType::I64);
    let values = [-5_i64, -1, 0, 3];
    let keys: Vec<Vec<u8>> = values
        .iter()
        .map(|v| idx.encode_index_key(&RecordValue::I64(*v), &pk(0)).unwrap())
        .collect();
    for pair in keys.windows(2) {
        assert!(pair[0] < pair[1], "byte order must match numeric order");
    }
}

#[test]
fn u64_uses_plain_big_endian_order() {
    let idx = index(FieldType::U64);
    let values = [0_u64, 1, i64::MAX as u64, (i64::MAX as u64) + 1, u64::MAX];
    let keys: Vec<Vec<u8>> = values
        .iter()
        .map(|v| idx.encode_index_key(&RecordValue::U64(*v), &pk(0)).unwrap())
        .collect();
    assert_eq!(field_component(&keys[0]), &0_u64.to_be_bytes());
    assert_eq!(field_component(&keys[4]), &u64::MAX.to_be_bytes());
    for pair in keys.windows(2) {
        assert!(pair[0] < pair[1], "byte order must match unsigned order");
    }
    for value in values {
        let key = idx
            .encode_index_key(&RecordValue::U64(value), &pk(42))
            .unwrap();
        let (decoded, decoded_pk) = idx.decode_index_key(&key).unwrap();
        assert_eq!(decoded, RecordValue::U64(value));
        assert_eq!(decoded_pk, pk(42));
    }
}

#[test]
fn f64_byte_order_equals_numeric_order() {
    let idx = index(FieldType::F64);
    let neg = idx
        .encode_index_key(&RecordValue::F64(-1.0), &pk(0))
        .unwrap();
    let zero = idx
        .encode_index_key(&RecordValue::F64(0.0), &pk(0))
        .unwrap();
    let pos = idx
        .encode_index_key(&RecordValue::F64(1.0), &pk(0))
        .unwrap();
    assert!(neg < zero, "-1.0 must sort before 0.0");
    assert!(zero < pos, "0.0 must sort before 1.0");
}

#[test]
fn i64_min_and_max_round_trip() {
    let idx = index(FieldType::I64);
    for value in [i64::MIN, i64::MAX, -1, 0, 1] {
        let key = idx
            .encode_index_key(&RecordValue::I64(value), &pk(42))
            .unwrap();
        let (decoded, decoded_pk) = idx.decode_index_key(&key).unwrap();
        assert_eq!(decoded, RecordValue::I64(value));
        assert_eq!(decoded_pk, pk(42));
    }
    // I64::MIN must sort below I64::MAX in byte order.
    let lo = idx
        .encode_index_key(&RecordValue::I64(i64::MIN), &pk(0))
        .unwrap();
    let hi = idx
        .encode_index_key(&RecordValue::I64(i64::MAX), &pk(0))
        .unwrap();
    assert!(lo < hi);
}

#[test]
fn text_empty_round_trips_and_sorts_first() {
    let idx = index(FieldType::Text);
    let empty = idx
        .encode_index_key(&RecordValue::Text(String::new()), &pk(0))
        .unwrap();
    // Empty text encodes to just the 2-byte terminator (self-delimiting).
    assert_eq!(field_component(&empty), &[ESC, ESC_TERM]);
    let (decoded, _) = idx.decode_index_key(&empty).unwrap();
    assert_eq!(decoded, RecordValue::Text(String::new()));
    // Empty sorts before any non-empty value.
    let a = idx
        .encode_index_key(&RecordValue::Text("a".into()), &pk(0))
        .unwrap();
    assert!(empty < a);
}

#[test]
fn text_prefix_sorts_before_longer_and_truncates_at_64() {
    let idx = index(FieldType::Text);
    let ab = idx
        .encode_index_key(&RecordValue::Text("ab".into()), &pk(0))
        .unwrap();
    let abc = idx
        .encode_index_key(&RecordValue::Text("abc".into()), &pk(0))
        .unwrap();
    assert!(ab < abc, "prefix 'ab' must sort before 'abc'");
    // 70-byte string is truncated to its first 64 bytes in the key.
    let long = "x".repeat(70);
    let key = idx
        .encode_index_key(&RecordValue::Text(long), &pk(0))
        .unwrap();
    let (decoded, _) = idx.decode_index_key(&key).unwrap();
    assert_eq!(decoded, RecordValue::Text("x".repeat(MAX_INDEXED_BYTES)));
}

#[test]
fn text_with_embedded_nul_round_trips() {
    let idx = index(FieldType::Text);
    let value = "a\u{0}b".to_string();
    let key = idx
        .encode_index_key(&RecordValue::Text(value.clone()), &pk(7))
        .unwrap();
    let (decoded, decoded_pk) = idx.decode_index_key(&key).unwrap();
    assert_eq!(decoded, RecordValue::Text(value));
    assert_eq!(decoded_pk, pk(7));
}

#[test]
fn bool_and_timestamp_round_trip_and_order() {
    let bidx = index(FieldType::Bool);
    let f = bidx
        .encode_index_key(&RecordValue::Bool(false), &pk(0))
        .unwrap();
    let t = bidx
        .encode_index_key(&RecordValue::Bool(true), &pk(0))
        .unwrap();
    assert!(f < t);
    assert_eq!(
        bidx.decode_index_key(&t).unwrap().0,
        RecordValue::Bool(true)
    );

    let tidx = index(FieldType::Timestamp);
    let lo = tidx
        .encode_index_key(&RecordValue::Timestamp(1), &pk(0))
        .unwrap();
    let hi = tidx
        .encode_index_key(&RecordValue::Timestamp(1_000_000), &pk(0))
        .unwrap();
    assert!(lo < hi);
    assert_eq!(
        tidx.decode_index_key(&hi).unwrap().0,
        RecordValue::Timestamp(1_000_000)
    );
}

#[test]
fn scan_prefix_is_key_without_pk() {
    let idx = index(FieldType::I64);
    let key = idx.encode_index_key(&RecordValue::I64(99), &pk(5)).unwrap();
    let prefix = idx.encode_scan_prefix(&RecordValue::I64(99)).unwrap();
    assert!(key.starts_with(&prefix));
    assert_eq!(prefix.len(), key.len() - 8);
}

#[test]
fn fail_closed_on_short_key() {
    let idx = index(FieldType::I64);
    let error = idx.decode_index_key(&[0x10, 0x00, 0x00]).unwrap_err();
    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
}

#[test]
fn fail_closed_on_type_mismatch_and_null() {
    let idx = index(FieldType::I64);
    // Wrong-typed value.
    let mismatch = idx
        .encode_index_key(&RecordValue::Text("x".into()), &pk(0))
        .unwrap_err();
    assert_eq!(mismatch.code, "CALYX_INVALID_ARGUMENT");
    // NULL is unindexable.
    let null = idx
        .encode_index_key(&RecordValue::Null, &pk(0))
        .unwrap_err();
    assert_eq!(null.code, "CALYX_INVALID_ARGUMENT");
    // Negative timestamp fails closed.
    let tidx = index(FieldType::Timestamp);
    let neg = tidx
        .encode_index_key(&RecordValue::Timestamp(-1), &pk(0))
        .unwrap_err();
    assert_eq!(neg.code, "CALYX_INVALID_ARGUMENT");
    // Non-finite float fails closed.
    let fidx = index(FieldType::F64);
    let nan = fidx
        .encode_index_key(&RecordValue::F64(f64::NAN), &pk(0))
        .unwrap_err();
    assert_eq!(nan.code, "CALYX_INVALID_ARGUMENT");
}

#[test]
fn fail_closed_on_corrupt_text_escape() {
    let idx = index(FieldType::Text);
    // Prefix + lone 0x00 followed by an invalid escape trailer (0x02).
    let mut key = idx
        .encode_scan_prefix(&RecordValue::Text("a".into()))
        .unwrap();
    // Replace the terminator's second byte with an invalid escape.
    let last = key.len() - 1;
    key[last] = 0x02;
    key.extend_from_slice(pk(0).as_bytes());
    let error = idx.decode_index_key(&key).unwrap_err();
    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
}

proptest! {
    #[test]
    fn prop_i64_round_trip_and_order(a in any::<i64>(), b in any::<i64>(), pk_val in any::<u64>()) {
        let idx = index(FieldType::I64);
        let ka = idx.encode_index_key(&RecordValue::I64(a), &pk(pk_val)).unwrap();
        let (da, dpk) = idx.decode_index_key(&ka).unwrap();
        prop_assert_eq!(da, RecordValue::I64(a));
        prop_assert_eq!(dpk, pk(pk_val));
        // Ordering with a fixed pk reflects numeric order.
        let ka0 = idx.encode_index_key(&RecordValue::I64(a), &pk(0)).unwrap();
        let kb0 = idx.encode_index_key(&RecordValue::I64(b), &pk(0)).unwrap();
        prop_assert_eq!(a < b, ka0 < kb0);
        prop_assert_eq!(a == b, ka0 == kb0);
    }

    #[test]
    fn prop_u64_round_trip_and_order(a in any::<u64>(), b in any::<u64>(), pk_val in any::<u64>()) {
        let idx = index(FieldType::U64);
        let ka = idx.encode_index_key(&RecordValue::U64(a), &pk(pk_val)).unwrap();
        let (da, dpk) = idx.decode_index_key(&ka).unwrap();
        prop_assert_eq!(da, RecordValue::U64(a));
        prop_assert_eq!(dpk, pk(pk_val));
        let ka0 = idx.encode_index_key(&RecordValue::U64(a), &pk(0)).unwrap();
        let kb0 = idx.encode_index_key(&RecordValue::U64(b), &pk(0)).unwrap();
        prop_assert_eq!(a < b, ka0 < kb0);
        prop_assert_eq!(a == b, ka0 == kb0);
    }

    #[test]
    fn prop_f64_round_trip_and_order(a in any::<f64>(), b in any::<f64>()) {
        prop_assume!(a.is_finite() && b.is_finite());
        let idx = index(FieldType::F64);
        let ka = idx.encode_index_key(&RecordValue::F64(a), &pk(1)).unwrap();
        let (da, _) = idx.decode_index_key(&ka).unwrap();
        prop_assert_eq!(da, RecordValue::F64(a));
        let ka0 = idx.encode_index_key(&RecordValue::F64(a), &pk(0)).unwrap();
        let kb0 = idx.encode_index_key(&RecordValue::F64(b), &pk(0)).unwrap();
        prop_assert_eq!(a < b, ka0 < kb0);
    }

    #[test]
    fn prop_text_round_trips_truncated(s in ".{0,200}", pk_val in 1u64..u64::MAX) {
        let idx = index(FieldType::Text);
        let key = idx.encode_index_key(&RecordValue::Text(s.clone()), &pk(pk_val)).unwrap();
        let (decoded, dpk) = idx.decode_index_key(&key).unwrap();
        let expected = truncate_utf8(&s).to_string();
        prop_assert_eq!(decoded, RecordValue::Text(expected));
        prop_assert_eq!(dpk, pk(pk_val));
    }

    #[test]
    fn prop_bytes_round_trips_truncated(bytes in proptest::collection::vec(any::<u8>(), 0..200)) {
        let idx = index(FieldType::Bytes);
        let key = idx.encode_index_key(&RecordValue::Bytes(bytes.clone()), &pk(3)).unwrap();
        let (decoded, _) = idx.decode_index_key(&key).unwrap();
        let expected = bytes[..bytes.len().min(MAX_INDEXED_BYTES)].to_vec();
        prop_assert_eq!(decoded, RecordValue::Bytes(expected));
    }
}
