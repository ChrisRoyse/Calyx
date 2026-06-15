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
  - MXFP4 falls back to MXFP8 when required;
  - binary/PQ requests fail safe through higher-precision fallback when measured recall breaches the declared delta;
  - MRL lenses store a truncated and unit-renormalized prefix at `truncate_dim`.
- Aster `VaultStore::get` remains compatible with compressed `slot_XX` rows by reading the raw sidecar if the active slot CF row is not a raw `SlotVector`.

The compressed slot envelope is binary and starts with `COMPRESSED_SLOT_TAG` (`16`). The envelope records codec, quant level, raw dimension, stored dimension, fallback flag, Matryoshka truncation flag, scale, seed id, and payload bytes.

PQ policy is accepted as a declared manifest policy, but because no trained codebook artifact is part of PH74 T03, the write path fails safe to TurboQuant Bits3p5 and records the fallback in the slot compression report. A trained PQ codebook artifact should be added by a later Sextant/Forge issue before storing real PQ codes.

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
5. Call `VaultStore::get` at the post-compression snapshot and verify the returned slot vector is the original 128-dimensional raw dense vector from `slot_03.raw`.
6. Compare `raw_bytes_total` and `stored_bytes_total` from the compression reports; stored bytes must be lower for both compressed slots.
7. Run `calyx panel status --vault "$CALYX_FSV_ROOT/panel-status-vault"` and verify the status JSON reports both `turbo_quant` and `mx_fp4` per-slot policies.

Manual edge readbacks:

- Empty batch returns `CALYX_VECTOR_COMPRESSION_EMPTY`.
- Matryoshka invalid/zero prefixes fail closed with `CALYX_VECTOR_COMPRESSION_INVALID`.
- Non-finite probe queries fail closed with `CALYX_VECTOR_COMPRESSION_INVALID` before recall scoring.
- Recall breach records `fallback_reason` and changes the stored codec away from the originally unsafe policy.

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
