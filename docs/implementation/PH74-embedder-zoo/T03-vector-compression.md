# PH74 T03 - Vector-Side Compression

Issue: #790

## Implementation

PH74 T03 wires registry lens manifest compression policy into panel slots and stored slot rows:

- `QuantPolicy` now has explicit `turbo_quant` and `mx_fp4` variants in addition to `pq`, `float8`, `binary`, and `none`.
- `LensSpec` and `LensForgeManifest` carry `quant_default`, optional `truncate_dim`, and `recall_delta`.
- Registry-backed panel slots inherit `LensSpec.quant_default` during `apply_panel_template`.
- `calyx panel status --vault <dir>` includes the per-slot `quant` field in the JSON status output.
- `calyx-registry::compression` provides the write path for dense slot batches:
  - raw f32 bytes are written to `slot_XX.raw`;
  - compressed bytes are written to `slot_XX`;
  - TurboQuant uses deterministic `(lens_id, cx_id)` rotation seeds;
  - Scalar INT8 uses the dedicated `Bits8` codec;
  - MXFP4/MXFP8 use explicit microscaling codecs selected by policy;
  - binary or PQ requests fail closed when measured recall breaches the declared delta or when the required trained PQ codebook is absent;
  - MRL lenses store a truncated and unit-renormalized prefix at `truncate_dim`.
- Aster `VaultStore::get` fails closed on compressed `slot_XX` rows; callers must use the compression-aware decode/readback path instead of silently reading a raw sidecar.

The compressed slot envelope is binary and starts with `COMPRESSED_SLOT_TAG` (`16`). The envelope records codec, quant level, raw dimension, stored dimension, Matryoshka truncation flag, scale, seed id, and payload bytes.

PQ policy is accepted as a declared manifest policy, but because no trained codebook artifact is part of PH74 T03, the write path fails closed rather than storing substitute TurboQuant bytes. A trained PQ codebook artifact should be added by a later Sextant/Forge issue before storing real PQ codes.

## FSV Recipe

Run on aiwonder from `/home/croyse/calyx/repo`:

```bash
export CALYX_FSV_ROOT=/home/croyse/calyx/tmp/issue790-fsv-$(date -u +%Y%m%d-%H%M%S)
cargo test -p calyx-registry --test issue790_vector_compression_fsv -- --nocapture
cargo build -p calyx-cli
```

Manual source-of-truth readback for the durable multi-lens vault case:

1. Read the `slot_03` CF row for the first `CxId`; byte `0` must be `16`.
2. Read the matching `slot_03.raw` CF row; byte `0` must be `0` (raw dense `SlotVector`).
3. Decode the compressed envelope and verify `raw_dim=128`, `stored_dim=64`, `codec=turbo_quant_bits3p5`, and `truncated=true`.
4. Read the companion `slot_04` CF row and verify its envelope is `codec=mx_fp4`, `raw_dim=128`, and `stored_dim=128`.
5. Call `VaultStore::get` at the post-compression snapshot and verify it returns the compressed-slot fail-closed error instead of masking the active compressed row with `slot_03.raw`.
6. Compare `raw_bytes_total` and `stored_bytes_total` from the compression reports; stored bytes must be lower for both compressed slots.
7. Run `calyx panel status --vault "$CALYX_FSV_ROOT/panel-status-vault"` and verify the status JSON reports both `turbo_quant` and `mx_fp4` per-slot policies.

Manual edge readbacks:

- Empty batch returns `CALYX_VECTOR_COMPRESSION_EMPTY`.
- Matryoshka invalid/zero prefixes fail closed with `CALYX_VECTOR_COMPRESSION_INVALID`.
- Non-finite probe queries fail closed with `CALYX_VECTOR_COMPRESSION_INVALID` before recall scoring.
- Recall breach fails closed and writes no substitute compressed slot row.

## Gate

Use the normal aiwonder merge gate:

```bash
cargo fmt --all -- --check
scripts/linecount.sh
cargo check --workspace
cargo check -p calyx-registry --features candle-cuda
cargo clippy --workspace --tests -- -D warnings
cargo test --workspace -- --nocapture
```

Attach the FSV root, CF byte readbacks, CLI panel status readback, recall deltas, and codec selections to the issue/PR before merging.
