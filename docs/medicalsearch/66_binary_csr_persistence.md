# Binary CSR Persistence for PlainGraph

Issue: #1210

Status: complete FSV for the local physical-vault proof path and the real
aiwonder biomedical graph vault.

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

## Real Biomedical Graph FSV

Remote source of truth:

```text
aiwonder:/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J
vault name: corpus-anchored-869-20260625T080546Z
collection: default
```

Remote command:

```text
CALYX_HOME=/home/croyse/calyx \
  ./target/release/calyx materialize-graph-csr \
  corpus-anchored-869-20260625T080546Z \
  --collection default
```

Remote FSV artifacts:

```text
/home/croyse/calyx/fsv/issue1210-binary-csr-real-20260704T101500Z
/home/croyse/calyx/fsv/issue1210-binary-csr-real-20260704T101500Z/issue1210_real_fsv_summary.json
```

Observed real-vault readback:

- git head: `9f105f4be07b08de18887597407fbfe9afd34b5f`
- status: `ok`
- prior CSR before-state: stale manifest v2 rejected as
  `CALYX_GRAPH_CORRUPT_ROW`
- source of truth: physical Graph CF latest readback through
  `PhysicalPlainGraph::read_csr`, `assoc_graph`, and independent node/edge key
  enumeration
- old JSON baseline bytes: `157072071`
- binary CSR bytes: `63235522`
- bytes saved: `93836549`
- size ratio vs old JSON baseline: `0.4025892165132272`
- binary less than half old JSON: `true`
- CSR SHA-256:
  `7c7a2928e12b4200d5c6c287008f982286490f8e003f4f6b9ee645331249ffd0`
- CSR blake3:
  `df9d23867f8feafd9a9ef15bb25207d60d4b000e67ba8b9cfd106b486c178662`
- nodes: `198993`
- CSR edges: `2435817`
- association edge count: `2435817`
- physical node keys: `198993`
- physical edge-out keys: `2435817`
- edge weight decode policy: `explicit-weight-or-legacy-unit`
- explicit weighted edges: `0`
- legacy unit-weight edges: `2435817`
- elapsed materialize/readback time: `81221 ms`

The real vault required two hardening fixes discovered only by FSV:

- stale persisted CSR manifests are recorded as before-state evidence and then
  rebuilt instead of blocking materialization
- legacy unweighted graph edge payloads are upgraded through an explicit,
  counted unit-weight policy; malformed explicit weights still fail closed
- the materializer builds the projection from physical graph range scans instead
  of `scan edge keys -> per-edge point read`, which avoided the real-vault CPU
  wall and completed the readback in about 81 seconds

## Boundary

This is association-substrate storage hardening. It improves persisted graph
size/readback behavior for mining, but does not itself validate any biomedical
hypothesis or clinical claim.
