# PH74 T04 - Anneal Conversion-Target Flywheel

PH74 T04 closes the loop between Assay deficits and the conversion factory.
When Anneal sees a panel sufficiency deficit, `CandidateLens::Commission`
now carries ranked `ConversionTarget` entries:

- `hf_id`: the LensForge/Hugging Face model to convert.
- `modality`: the modality the target is expected to cover.
- `axis`: the missing measurement axis, such as `protein_sequence`.
- `formats`: the supported LensForge formats for that target.
- `expected_bits`: deterministic expected bit gain, capped at the measured deficit.

The initial catalog mirrors `tools/lensforge/registry.yaml` and covers text,
audio, image, protein, DNA, and molecule targets. Ranking is deterministic:
expected bits desc, then `hf_id`, then axis. The differentiation gate still
enforces the PH29 contract before any hot-add: candidates with less than
0.05 bits or more than 0.6 pairwise correlation are rejected.

## Hot-Add Path

`RegistryHotAdder` now handles commissioned candidates. For the top
conversion target it:

1. Writes a deterministic commissioned artifact under the configured artifact
   directory.
2. Registers the artifact in `calyx-registry` with the target modality and axis.
3. Adds the resulting slot to the panel with TurboQuant default storage policy.
4. Lets `ProposeLens` re-read Assay sufficiency; if sufficiency does not
   increase, it rolls back the substrate and restores the prior panel.

Budget rejection remains fail-closed. If the Anneal substrate returns
`BudgetExhausted`, the proposal outcome is `SubstrateReverted` with the
change id retained; no panel mutation or ledger admission row is written.

## FSV

Run on aiwonder:

```bash
cd /home/croyse/calyx/repo
. ./env.sh
export CALYX_FSV_ROOT=/home/croyse/calyx/tmp/issue791-fsv-$(date -u +%Y%m%d-%H%M%S)
cargo test -p calyx-anneal --test issue791_conversion_flywheel_fsv -- --nocapture
cat "$CALYX_FSV_ROOT/summary.json"
cat "$CALYX_FSV_ROOT/physical-files.txt"
cat "$CALYX_FSV_ROOT/BLAKE3SUMS.txt"
```

Source of truth:

- Aster ledger CF rows under `$CALYX_FSV_ROOT/vault/cf/ledger`.
- Commissioned artifact bytes under `$CALYX_FSV_ROOT/factory-artifacts`.
- The `summary.json` readback that includes panel before/after, target, probe
  vector, and edge outcomes.

Expected happy-path readback:

- target `hf_id` is `facebook/esm2_t6_8M_UR50D`.
- target modality is `protein`.
- target axis is `protein_sequence`.
- panel gains one active protein slot.
- commissioned artifact exists and hashes in `BLAKE3SUMS.txt`.
- Anneal ledger has `LensAdmitted` followed by duplicate `LensRejected`.

Edges:

- zero deficit returns `NoDeficit` and leaves ledger rows unchanged.
- duplicate/high-correlation candidate is rejected by `TooCorrelated`.
- over-budget candidate returns `SubstrateReverted(BudgetExhausted)` and leaves
  ledger rows unchanged.
