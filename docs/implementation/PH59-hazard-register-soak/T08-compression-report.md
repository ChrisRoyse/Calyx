# PH59 · T08 — doc-23 compression_report(vault)

**Issue:** #613
**Source binding:** PRD `23 §4.4`, `23 §4.5`, doc `18 §4`

## Capability

`calyx_forge::compression_report(input)` now builds the doc-23 honest-numbers
report for an already-quantized vault snapshot. The report aggregates:

- bits/channel per slot from `QuantLevel`
- achieved cosine distortion vs the TurboQuant floor
- slot and kernel bytes saved plus compression ratios
- kernel-only recall before/after and delta
- Assay bits delta and Ward FAR/FRR delta per slot
- meaning-compression yield from saved storage and retained Assay bits

The API fails closed with `CALYX_QUANT_INTELLIGENCE_LOSS` when the measured
intelligence contract regresses beyond the supplied bounds. Malformed report
inputs fail as `CALYX_FORGE_QUANT_ERROR`.

## Readback Artifact

Manual FSV persists a PH59 artifact:

```json
{
  "schema_version": 1,
  "surface": "compression-report",
  "artifact_kind": "ph59.compression-report.v1",
  "source_of_truth": "PH59 compression report artifact",
  "report": { "...": "CompressionReport" }
}
```

`calyx readback compression-report --artifact <json> [--field <path>]` reads the
artifact bytes and prints the selected value with artifact hash/length metadata.

## FSV

On aiwonder:

```bash
CALYX_ISSUE613_FSV_ROOT=/home/croyse/calyx/data/fsv-issue613-compression-report-<ts> \
  cargo test -p calyx-cli --test compression_report_readback \
  issue613_compression_report_full_doc23_fsv -- --ignored --nocapture
```

The FSV test writes real TurboQuant-encoded slot bytes under the synthetic vault,
persists the compression-report artifact, reads it through the CLI readback
surface, and records ≥3 fail-closed edge cases with before/after file state.
