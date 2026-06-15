# PH74 T05 - Embedder-Zoo Stage-Exit FSV

Issue: #792
Stage: S21 Embedder Zoo and Lens Conversion Factory

## Implementation

T05 is the PH74 stage-exit rollup. It exercises the merged T01-T04 surfaces in
one workflow:

1. Build a deterministic model-id list with real Hugging Face IDs across text,
   image, audio, protein, DNA, and molecule modalities.
2. Convert each target through `commission_lens`, producing physical factory
   artifacts under the FSV root.
3. Register the commissioned artifacts as frozen registry lenses.
4. Run per-slot capability cards with Assay-backed signal bits.
5. Apply the capability gate: seven lenses are admitted, one low-signal lens is
   parked, and one duplicate candidate is retired by the correlation contract.
6. Build a durable Aster vault with the admitted panel and fixture
   multi-modal constellations.
7. Persist Assay CF rows and Loom XTerm rows, then write the DDA
   `AbundanceReport` into `vault/intelligence/abundance.json`.

The CLI now exposes the stage-exit abundance readback:

```bash
${CARGO_TARGET_DIR:-target}/debug/calyx intelligence abundance --vault <vault>
```

The command reads the vault source of truth at
`<vault>/intelligence/abundance.json` and prints the JSON report.

## FSV Recipe

Run on aiwonder from `/home/croyse/calyx/repo`:

```bash
export CALYX_FSV_ROOT=/home/croyse/calyx/tmp/issue792-fsv-$(date -u +%Y%m%d-%H%M%S)
cargo test -p calyx-registry --test embedder_zoo_fsv -- --nocapture
cargo build -p calyx-cli
${CARGO_TARGET_DIR:-target}/debug/calyx intelligence abundance --vault "$CALYX_FSV_ROOT/vault" \
  > "$CALYX_FSV_ROOT/cli-abundance.json"
cat "$CALYX_FSV_ROOT/summary.json"
cat "$CALYX_FSV_ROOT/BLAKE3SUMS.txt"
```

## Source Of Truth

- Durable Aster vault under `$CALYX_FSV_ROOT/vault`.
- Assay CF rows in that vault.
- Panel and registry assets persisted beside the vault.
- Loom XTerm CF under `$CALYX_FSV_ROOT/xterm-cf`.
- Commissioned factory artifacts under `$CALYX_FSV_ROOT/factory-artifacts`.
- Capability gate ledger under `$CALYX_FSV_ROOT/capability-ledger`.
- Abundance readback at `$CALYX_FSV_ROOT/vault/intelligence/abundance.json`.

Expected evidence:

- `conversion-readback.json` lists at least eight converted real model IDs over
  at least four modalities.
- `panel-decisions.json` shows `admit=7`, `park=1`, and `retire=1`.
- `assay-readback.json` shows image/protein cross-modal pair gain at or above
  `0.05` bits.
- `xterm-readback.json` materializes the admitted panel cross-terms.
- `summary.json` reports `n_eff` above the duplicate-adjusted expectation.
- `footprint.json` records RSS and `nvidia-smi` readback for aiwonder.
- `cli-abundance.json` matches `vault/intelligence/abundance.json`.
