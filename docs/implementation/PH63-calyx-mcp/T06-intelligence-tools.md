# PH63 · T06 — Intelligence extraction tool group

| Field | Value |
|---|---|
| **Phase** | PH63 — calyx-mcp (stdio embedded tool surface) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-mcp` |
| **Files** | `crates/calyx-mcp/src/tools/intelligence.rs` (≤500) |
| **Depends on** | T04, PH28 (KSG MI/bits), PH30 (abundance), PH32 (kernel), PH37 (guard), PH47 (propose_lens) |
| **Axioms** | A7, A8, A10, A12, A17 |
| **PRD** | `dbprdplans/14 §2` (intelligence extraction group), `dbprdplans/18 §6` |

## Goal

Register the five intelligence-extraction tools: `abundance`, `bits`, `kernel`,
`guard.calibrate`/`guard.check`, and `propose_lens`. These tools let an agent
audit measurement quality and act on gaps without any code. The error remediation
strings are verbatim from PRD 18 §6 so the agent can self-correct.

## Build (checklist of concrete, code-level steps)

- [ ] **`calyx.abundance`** schema and impl:
  - Schema: `{"vault": string(required)}`
  - Use when: `"DDA report: N, C(N,2), materialized, n_eff, DPI ceiling"`
  - Returns: `{"n":<cx_count>,"pairs":<c_n_2>,"materialized":<m>,"n_eff":<f>,
    "dpi_ceiling":<f>,"panel_size":<slots>}`

- [ ] **`calyx.bits`** schema and impl:
  - Schema: `{"vault": string(required), "anchor": string(required,
    description:"anchor kind e.g. test_pass"), "explain": boolean(optional)}`
  - Use when: `"per-lens signal + panel sufficiency + deficit attribution"`
  - Returns: `BitsReport` JSON with `panel_sufficiency`, `n`, `per_slot` array
  - `CALYX_ASSAY_INSUFFICIENT_SAMPLES` (n<50) → `error.data.remediation:
    "anchor ≥50 outcomes first"` (verbatim)
  - `CALYX_ASSAY_LOW_SIGNAL` for any slot < 0.05 bits → `remediation:
    "park/retire lens"` (verbatim)
  - `CALYX_ASSAY_REDUNDANT` for pair corr > 0.6 → `remediation:
    "drop duplicate lens"` (verbatim)

- [ ] **`calyx.kernel`** schema and impl:
  - Schema: `{"vault": string(required), "anchor": string(optional),
    "rebuild": boolean(optional, default:false)}`
  - Use when: `"build/get the grounding kernel + recall + grounding gaps"`
  - Returns: `{"kernel_size":<n>,"recall":0.96,"total_cx":<m>,
    "kernel_cx_ids":["…"],"grounding_gaps":[…]}`
  - `CALYX_KERNEL_UNGROUNDED` → `error.data.remediation: "add anchors
    (grounding_gaps)"` (verbatim)

- [ ] **`calyx.guard.calibrate`** schema and impl:
  - Schema: `{"vault": string(required), "domain": string(required),
    "set": string(required, description:"JSONL path to calibration corpus"),
    "target_far": number(required, description:"target false-accept rate e.g.
    0.01")}`
  - Use when: `"calibrate the Gτ boundary for a domain"`
  - Returns: `GuardProfile` JSON with `tau`, `far`, `frr`, `n_corpus`

- [ ] **`calyx.guard.check`** schema and impl (separate from calibrate):
  - Schema: `{"vault": string(required), "cx_id": string(optional),
    "text": string(optional, description:"check raw text without storing")}`
  - Use when: `"apply the Gτ boundary to a constellation or text"`
  - Returns: `{"verdict":"pass"|"ood","tau":0.12,"distance":0.08}`
  - `CALYX_GUARD_PROVISIONAL` if not calibrated

- [ ] **`calyx.propose_lens`** schema and impl:
  - Schema: `{"vault": string(required), "anchor": string(required)}`
  - Use when: `"ask Calyx what lens would close a sufficiency gap"`
  - Returns: `{"name":"…","rationale":"…","predicted_bits_gain":0.3,
    "runtime_hint":"tei-http","estimated_cost":"…"}`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `bits` with n=30 → `error.data.calyx_code:"CALYX_ASSAY_INSUFFICIENT_SAMPLES"`,
  `error.data.remediation:"anchor ≥50 outcomes first"` (byte-exact match)
- [ ] unit: `bits` with a low-signal lens (< 0.05 bits) → that slot's entry in
  `per_slot` has `"bits":< 0.05`; `CALYX_ASSAY_LOW_SIGNAL` in error data
- [ ] unit: `abundance` on a vault with 100 cx and 2 slots → `n=100`,
  `pairs=4950`, `panel_size=2`
- [ ] unit: `guard.calibrate` returns a `tau` value in (0.0, 1.0); `guard.check`
  after calibration returns `verdict` present and `distance` ≥ 0.0
- [ ] edge: `kernel` with no anchors → `CALYX_KERNEL_UNGROUNDED`; `propose_lens`
  with no anchor kind that has sufficient samples → still returns a proposal
  (not an error); `abundance` on empty vault → `n=0`, `pairs=0`
- [ ] fail-closed: `guard.check` before calibration → `CALYX_GUARD_PROVISIONAL`,
  not a silent `"verdict":"pass"`; `bits` with `anchor:"nonexistent_kind"` →
  `CALYX_ASSAY_INSUFFICIENT_SAMPLES` (0 samples for that kind)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the Assay CF entries at `<vault.calyx>/cf/assay/<anchor_kind_hex>`
  after a `bits` MCP call on aiwonder; the GuardProfile CF entry after a
  `guard.calibrate` call
- **Readback:** after `tools/call bits {"vault":"mcp-test","anchor":"test_pass"}`,
  run `calyx readback --cf-row <vault.calyx> --cf assay --key <anchor_hex>` on
  aiwonder → non-empty hex dump containing signal bytes; after `guard.calibrate`,
  `calyx readback --cf-row <vault.calyx> --cf guard --key <domain_hex>` →
  GuardProfile bytes present
- **Prove:** `bits` response `per_slot[0].bits` is a finite positive float; the
  Assay CF hex dump is non-empty and changes when a new anchor is added
  (bits recomputed); `guard.calibrate` with a real corpus produces a `tau` in (0,1)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH63 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
