# Binary CSR Persistence for PlainGraph

Issue: #1210

Status: complete FSV for the local physical-vault proof path.

The persisted `PlainGraphCsr` stream now uses a binary columnar layout inside
the existing manifest-plus-segment framing:

- manifest row remains JSON and carries collection, snapshot, counts, segment
  count, byte total, and stream hash
- manifest version bumped to `4`
- stream starts with `CALYXCSR`
- node ids and edge destinations are raw 16-byte `CxId` values
- offsets are fixed little-endian `u64`
- edge weights are raw `f32` little-endian bytes
- edge types are dictionary-encoded with `u32` indexes

The reader still reassembles ordered segments, verifies byte count and blake3,
then decodes. Unsupported manifest versions, truncated segments, hash mismatch,
count mismatch, invalid edge types, and invalid weights fail closed as
`CALYX_GRAPH_CORRUPT_ROW`.

## Verification

Focused gates:

```text
cargo fmt --check
bash scripts/linecount.sh
cargo test -p calyx-aster plain_graph::csr -- --nocapture
cargo test -p calyx-aster plain_graph:: -- --nocapture
cargo test -p calyx-aster large_csr_projection_shards_into_segments_and_roundtrips -- --nocapture
cargo check -p calyx-aster
```

Physical FSV command:

```text
CALYX_FSV_ROOT=C:\code\Calyx-Dev\target\fsv\issue1210-binary-csr-20260704T075700Z \
  cargo test -p calyx-aster --test issue1210_binary_csr_fsv -- --nocapture
```

Physical FSV readback:

```text
target/fsv/issue1210-binary-csr-20260704T075700Z/issue1210_binary_csr_readback.json
```

Observed physical readback:

- source of truth: reopened durable Aster Graph CF through
  `PhysicalPlainGraph::read_csr_bytes` and `read_csr`
- binary magic: `CALYXCSR`
- binary CSR bytes: `481666`
- JSON baseline bytes: `1694623`
- binary less than half JSON: `true`
- binary CSR SHA-256:
  `9bc0e6da7c12f1e43116ca1c4f2f8303424b3e56eda4fa9daa362e002b0739e3`
- node count: `96`
- edge count: `19968`
- association edge count: `19968`
- edge cases: empty graph roundtrip, self-loop roundtrip, and `70000`
  distinct edge types roundtrip

The existing large in-crate sharding test still exercises segmentation after
the binary shrink: `segments=2 total_bytes=1269682`.

## Boundary

This is association-substrate storage hardening. It improves persisted graph
size/readback behavior for mining, but does not itself validate any biomedical
hypothesis or clinical claim.
