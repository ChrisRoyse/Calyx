# PH54 T07 - Native U64 Secondary-Index Field

Issue: #750
Date: 2026-06-14
Branch: `issue750-u64-secondary-index`

## Scope

PH54 secondary indexes previously had no native unsigned 64-bit field type. KV
and TimeSeries schema-less synthetic index keys therefore used big-endian bytes
for unsigned namespace, series, and timestamp values. That preserved physical
ordering, but it meant consumers had to query unsigned fields as `Bytes`, and
schema-full users could not declare a U64 secondary-index column.

This task adds native unsigned 64-bit support:

- `FieldType::U64` and `RecordValue::U64(u64)` are appended to their enums to
  avoid shifting existing serialized enum ordinals.
- BTree secondary indexes encode U64 fields as plain big-endian `u64` bytes.
  No sign flip is applied; unsigned lexical byte order already matches numeric
  order for fixed-width big-endian values.
- BTree decoding maps `FieldType::U64` back to `RecordValue::U64`.
- Relational schema validation, index maintenance, document schema validation,
  and inferred JSON values all recognize native U64 values.
- KV and TimeSeries schema-less/default synthetic index fields now emit
  `RecordValue::U64`; explicit `FieldType::Bytes` still uses the prior
  big-endian byte encoding for compatibility.
- Existing issue #460 FSV tests were updated from the prior Bytes workaround to
  native U64 query/readback expectations.

## Edge Coverage

The durable issue #750 FSV writes the boundary-rich set:

```text
[u64::MAX, 0, i64::MAX + 1, i64::MAX, 1]
```

The expected unsigned sort order is:

```text
[0, 1, 9223372036854775807, 9223372036854775808, 18446744073709551615]
```

The FSV covers two persisted collections in one durable vault:

- schema-less KV with a default BTree `ns` index
- schema-full relational data with `score: FieldType::U64` and a BTree `score`
  index

After flush, drop, and reopen, the FSV verifies:

- KV `u64::MAX` readback returns the expected value bytes.
- Relational row primary key `1` readback returns `RecordValue::U64(u64::MAX)`.
- Physical BTree keys decode to `RecordValue::U64` for every boundary value.
- The U64 field component bytes equal `value.to_be_bytes()`.
- Point queries for `RecordValue::U64(u64::MAX)` return the expected physical
  primary keys.

## aiwonder Gates

Targeted aiwonder gate `issue750_aiwonder_targeted_gate` passed:

```text
cargo fmt --all -- --check
500-line gate over touched Rust files
cargo check -p calyx-aster
cargo clippy -p calyx-aster -- -D warnings
cargo test -p calyx-aster index::btree -- --nocapture
cargo test -p calyx-aster layers::kv -- --nocapture
cargo test -p calyx-aster --test issue460_kv_unsigned_ns_index_fsv -- --nocapture
cargo test -p calyx-aster --test issue750_u64_index_fsv -- --nocapture
```

Workspace aiwonder gate `issue750_aiwonder_workspace_gate` passed:

```text
cargo check --workspace
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

Line-count gate after formatting:

```text
crates/calyx-aster/src/index/btree.rs                         497
crates/calyx-aster/src/index/btree_tests.rs                   316
crates/calyx-aster/src/layers/relational.rs                   491
crates/calyx-aster/src/layers/timeseries.rs                   456
crates/calyx-aster/tests/issue750_u64_index_fsv.rs            206
```

## Manual FSV Evidence

Source-of-truth root on aiwonder:

```text
/home/croyse/calyx/data/fsv-issue750-u64-index-20260614T0834Z
```

Artifact:

```text
/home/croyse/calyx/data/fsv-issue750-u64-index-20260614T0834Z/issue750-u64-index-fsv-artifact.json
```

Artifact BLAKE3:

```text
76138a4f3bf65315d7d2a26ff1c2f4a66e4a86bd71760dc0c4a30558b5d4e856
```

Artifact readback values:

```text
write_order:
  [18446744073709551615, 0, 9223372036854775808, 9223372036854775807, 1]
expected_unsigned_order:
  [0, 1, 9223372036854775807, 9223372036854775808, 18446744073709551615]
kv_schema_less_decoded_order:
  [0, 1, 9223372036854775807, 9223372036854775808, 18446744073709551615]
relational_schema_full_decoded_order:
  [0, 1, 9223372036854775807, 9223372036854775808, 18446744073709551615]
kv_max_ns_pk_hex:
  0310f2d5d4b4553f56ffffffffffffffff00016b
kv_max_ns_value_hex:
  76
rel_max_score_pk_hex:
  0000000000000001
rel_max_score_value:
  18446744073709551615
index_btree_sot:
  /home/croyse/calyx/data/fsv-issue750-u64-index-20260614T0834Z/vault/cf/index_btree
```

Manual SST byte readbacks:

- The KV `u64::MAX` BTree SST contains the U64 field component
  `ff ff ff ff ff ff ff ff`, followed by the persisted KV primary key.
- The relational `u64::MAX` BTree SST contains the U64 field component
  `ff ff ff ff ff ff ff ff`, followed by primary key `0000000000000001`.
- The relational value `1` BTree SST contains the U64 field component
  `00 00 00 00 00 00 00 01`.

IndexBtree SST BLAKE3 readbacks:

```text
00000000000000000001.sst       0193a69556ff8d94f90b35dffd4aaf0488b28018c013eae0bce166908808a1bc  1020 bytes
00000000000000000003-0002.sst  e868d3518ec74ee4aac1161c8049a9e0396ed6cc8fe4565960624d6491b26644  162 bytes
00000000000000000004-0002.sst  7332203b36babfa393102bfafd377be712813051687bd62cd2f0f3fe7adad172  138 bytes
00000000000000000005-0002.sst  501b2e8c6727b82195124ae45fdf51ad04cd31627e49441f342744e214f711e3  162 bytes
00000000000000000006-0002.sst  3e53da6d61fc29e0a3ebc59d355d4ad0a19dbec7df932ebfc5184205054244ac  138 bytes
00000000000000000007-0002.sst  882dcc4784f704c167e2f13a3a7b057c8ad1d359c4e2e6ad68a043c2fda37eff  162 bytes
00000000000000000008-0002.sst  13e1e097d9e3025f16eec6ffdfd6b56686647861f02f114bf42fc2bc08091698  138 bytes
00000000000000000009-0002.sst  38472b11fe8eaa40d66e8c1fa83ae38cb4628cfce4016d9729a96e25fcfdc89c  162 bytes
00000000000000000010-0002.sst  d6bc8fabf9f2bc8bb8128dc5bb0b441670ce8d840e5e5252de3c293baabc2b50  138 bytes
00000000000000000011-0002.sst  965148595d25fcbc1bd5fcbb74da120df262d0203fa8211559b1d071607bcc9c  162 bytes
00000000000000000012-0002.sst  e63926c1a617b8fce94a25e1a6a0bef5ad7322dd7194e8335f389d717a085e27  138 bytes
```
