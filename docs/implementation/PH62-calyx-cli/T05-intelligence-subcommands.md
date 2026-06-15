# PH62 · T05 — Intelligence subcommands (bits/kernel/guard/abundance)

| Field | Value |
|---|---|
| **Phase** | PH62 — calyx-cli (vault/lens/ingest/search/readback) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/cmd/intelligence.rs` (≤500) |
| **Depends on** | T04, PH28 (KSG MI/bits), PH30 (abundance report), PH32 (kernel-graph), PH37 (guard math) |
| **Axioms** | A7, A8, A10, A12, A17 |
| **PRD** | `dbprdplans/14 §2` (intelligence extraction group), `dbprdplans/18 §4` |

## Goal

Implement the five intelligence-extraction commands that expose the Assay, Lodestar,
Ward, and Anneal layers through the CLI. These are the commands that answer "which
lenses are earning their place", "what's the grounding kernel", "is the guard
calibrated", and "what panel change would close the gap". An agent uses these to
audit a vault's measurement quality without code.

## Build (checklist of concrete, code-level steps)

- [ ] `cmd/intelligence.rs` — `bits <vault> <anchor-kind> [--explain]`: calls
  `Calyx::bits(vault, anchor_kind)` → prints `BitsReport` as JSON:
  `{"anchor":"test_pass","panel_sufficiency":0.83,"n":142,"dpi_ceiling":3.1,
   "per_slot":[{"slot":0,"name":"gte-768","bits":0.72,"ci":[0.61,0.83],
   "estimator":"ksg","state":"active"}]}`;
  `CALYX_ASSAY_INSUFFICIENT_SAMPLES` when n < 50 → remediation `"anchor ≥50
  outcomes first"` (verbatim from PRD 18 §6)
- [ ] `cmd/intelligence.rs` — `kernel <vault> [--anchor <kind>] [--rebuild]`: calls
  `Calyx::build_kernel(vault, anchor)` → prints
  `{"kernel_size":<n>,"recall":0.96,"total_cx":<m>,
   "kernel_cx_ids":["…"],"grounding_gaps":[…]}`;
  `CALYX_KERNEL_UNGROUNDED` when kernel cannot be built → remediation `"add
  anchors (grounding_gaps)"`
- [ ] `cmd/intelligence.rs` — `guard <vault> <subcommand>` with three sub-forms:
  - `guard calibrate <vault> --domain <d> --set <jsonl> --target-far <f32>`:
    calls `Calyx::calibrate_guard` → prints `GuardProfile` JSON with `tau`,
    `far`, `frr`, calibration corpus size
  - `guard check <vault> --cx <cx_id> [--identity-cx <cx_id>]`: calls
    `Calyx::guard(vault, cx, identity_cx)` → prints `{"verdict":"pass"|"ood",
    "tau":0.12,"distance":0.08}`; `CALYX_GUARD_PROVISIONAL` when uncalibrated
  - `guard generate <vault> --candidate-text <s> [--identity-cx <cx_id>]`:
    calls `Calyx::guard_generate` (identity-locked generation gate)
- [ ] `cmd/intelligence.rs` — `abundance <vault>`: calls `Calyx::abundance` →
  prints `AbundanceReport` JSON:
  `{"n":<cx_count>,"pairs":<c_n_2>,"materialized":<m>,"n_eff":<f>,
   "dpi_ceiling":<f>,"panel_size":<slots>}`
- [ ] `cmd/intelligence.rs` — `propose-lens <vault> --anchor <kind>`: calls
  `Calyx::propose_lens(vault, anchor)` → prints `CandidateLens` JSON with
  `name`, `rationale`, `predicted_bits_gain`, `runtime_hint`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `bits` with n=30 anchors → `CALYX_ASSAY_INSUFFICIENT_SAMPLES` on
  stderr; remediation text = `"anchor ≥50 outcomes first"` (exact match)
- [ ] unit: `bits` with n=100 anchors on planted-signal data → `per_slot` contains
  a slot with `bits >= 0.05` and another with `bits < 0.05` (low-signal flagged)
- [ ] unit: `guard calibrate` with injection corpus → prints calibrated `tau`
  that blocks ≥99% of injections at the calibrated FAR
- [ ] unit: `abundance` on a vault with 100 constellations and 2 active slots →
  `n=100`, `pairs=4950`, `panel_size=2`
- [ ] edge: `guard check` before calibration → `CALYX_GUARD_PROVISIONAL`, exit 2;
  `kernel` with no anchors at all → `CALYX_KERNEL_UNGROUNDED`, exit 2;
  `bits` with no active slots → structured error, not panic
- [ ] fail-closed: `CALYX_ASSAY_LOW_SIGNAL` (< 0.05 bits) carries exact
  remediation `"park/retire lens"` in stderr JSON

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the Assay signal rows stored in `<vault.calyx>/cf/assay/<anchor_kind>`
  after a `bits` run; the kernel index in `<vault.calyx>/cf/kernel/`
- **Readback:** `calyx readback --hex <vault.calyx>/cf/assay/<anchor_kind_hex>`
  after running `calyx bits aiwonder-test test-pass`; the raw bytes contain the
  per-slot signal records; cross-check with a direct CF read using `xxd`
- **Prove:** signal bytes are non-empty; `bits` ≥ 0.05 for the embedded lens on a
  real aiwonder corpus with ≥50 anchors; `kernel` recall ≥ 0.95 on the kernel CF
  row read directly (PRD 14 §2: kernel recall ≥ 0.95·full on ≥3 real corpora);
  `guard calibrate` writes a `GuardProfile` CF entry readable via readback

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH62 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
