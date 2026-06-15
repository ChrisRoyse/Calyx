# PH70 - T08 - Polis civic-panel constellation/guard FSV

| Field | Value |
|---|---|
| Phase | PH70 - Intelligence validation on real corpora |
| Stage | S18 - Datasets & Intelligence FSV |
| Crate | `calyx-ward` |
| Files | `crates/calyx-ward/src/polis.rs` (<=500), `crates/calyx-ward/tests/polis_civic_fsv.rs` (<=500) |
| GitHub issue | `#611` |
| Depends on | PH69 T08 synthetic personas, PH38 Ward guard |

## Goal

Prove the privacy-safe Polis civic instantiation named in PRD `28` row 11. The
verifier evaluates planted synthetic persona pairs across the 21 civic slots,
uses Ward's per-slot `Gtau` guard with all 21 slots required, and records whether
the guard verdict produces the planted tie/no-tie outcome.

## Known I/O

The deterministic fixture uses signed scalar axes. For one-dimensional civic
slots, same-sign axes produce cosine `1.0`; opposite-sign axes produce cosine
`-1.0`. With tau `0.7`, the hand-computed expectations are:

- `tie-alpha-beta`: all 21 axes agree, tie passes
- `tie-gamma-delta`: all 21 axes agree, tie passes
- `reject-single-axis-07`: slot 7 disagrees, tie fails with failing slot `[7]`
- `reject-majority-shift`: slots 1-11 disagree, tie fails with failing slots
  `[1,2,3,4,5,6,7,8,9,10,11]`

Temporal slots from `civic-default` are explicitly excluded; the civic tie guard
is only over the 21 Polis axes.

## Fail-Closed Cases

- `CALYX_POLIS_EMPTY_PERSONA_SET`: no persona pairs
- `CALYX_POLIS_SLOT_COUNT_MISMATCH`: a persona does not have exactly 21 axes
- `CALYX_POLIS_INVALID_AXIS`: an axis is zero, NaN, or infinite
- `CALYX_POLIS_TIE_MISMATCH`: planted tie label does not match the guard outcome

The FSV test records before/after artifact state for each edge and writes no edge
proof artifact on failure.

## FSV

Run on aiwonder with a fresh root:

```bash
CALYX_ISSUE611_FSV_ROOT=/home/croyse/calyx/data/fsv-issue611-polis-civic-<ts> \
  cargo test -p calyx-ward --test polis_civic_fsv \
  issue611_polis_civic_guard_fsv_writes_readbacks -- --ignored --nocapture
```

Then manually read:

- `synthetic-persona-pairs.json`
- `polis-civic-proof.json`
- `issue611-fsv-readback.json`
- `BLAKE3SUMS.txt`
- edge directories under `edges/`

The source-of-truth proof must show `civic_slot_count=21`,
`all_expected_outcomes_match=true`, two passing tie pairs, two rejected non-tie
pairs, and the expected failing slot lists above.

## Done When

- `cargo check -p calyx-ward`, `cargo test -p calyx-ward`, and
  `cargo clippy -p calyx-ward --all-targets -- -D warnings` pass on aiwonder
- every `.rs` source/test file stays at or below 500 lines
- FSV evidence root is under `/home/croyse/calyx/data/fsv-issue611-*`
- manual readback confirms proof bytes, persona bytes, hash manifest, and all
  fail-closed edge cases
