# PH39 · T05 — Identity-slot injection → quarantine FSV

| Field | Value |
|---|---|
| **Phase** | PH39 — Identity-Locked Generation (Speaker / Style) |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/tests/identity_fsv.rs` (≤500) |
| **Depends on** | T04 (this phase) |
| **Axioms** | A12, A2, A16 |
| **PRD** | `dbprdplans/09 §5b` |

## Goal

Prove on aiwonder that a prompt injection designed to break persona lands outside
τ on the style slots and is quarantined — not silently accepted. The test must
use at least one real injection prompt from the on-disk injection corpus, not
only synthetic vectors. The `NoveltyRecord` with `status: Quarantined` must be
readable from the in-memory vault sink, confirming the routing.

## Build (checklist of concrete, code-level steps)

- [ ] Write `#[test] fn fsv_injection_breaks_style_quarantined`:
      - Load the style `IdentityProfile` with calibrated τ on the style slot
        (from `/home/croyse/calyx/data/identity_fsv/style_profile.json` on
        aiwonder; skip gracefully if absent)
      - Load matched style vector from
        `/home/croyse/calyx/data/identity_fsv/matched_style.npy`
      - Load one real injection text from
        `/home/croyse/calyx/data/injection_corpus/style_injection_01.txt`
      - Use `StyleLens` (real model on aiwonder; mock on dev)
      - Call `guard_generate()` with `novelty_action: Quarantine`,
        `high_stakes: false`
      - Assert `GenerateOutput::Novel { record }` where
        `record.status == Quarantined` and `record.action_taken == Quarantine`
      - Print `record.failing_verdicts` — show per-slot `(cos, tau, pass)` on
        the style slot
      - Assert `record.failing_verdicts.iter().any(|v| v.slot == "style" && !v.pass)`
- [ ] Write `#[test] fn fsv_in_persona_text_accepted`:
      - Load an in-persona text sample from
        `/home/croyse/calyx/data/identity_fsv/in_persona_01.txt`
      - Same profile and matched vecs as above
      - Call `guard_generate()`
      - Assert `GenerateOutput::Accepted { provenance_tag: "guarded:pass" }`
      - Print per-slot verdicts; assert all `pass == true`
- [ ] Write `#[test] fn fsv_quarantine_record_in_sink`:
      - Confirm `NoveltyRecord` is written to the `VaultSink`; call
        `novel_regions(since=0)` on the in-memory sink
      - Assert record present with `status: Quarantined`; print as JSON
      - `novel_id` is a non-nil UUID; `guard_id` matches the profile
- [ ] All tests: skip gracefully if data files absent (non-aiwonder dev)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `fsv_injection_breaks_style_quarantined` — asserts `Quarantined`;
      prints failing style-slot verdict
- [ ] unit: `fsv_in_persona_text_accepted` — asserts `Accepted`;
      prints `"guarded:pass"` provenance tag
- [ ] unit: `fsv_quarantine_record_in_sink` — record readable; all fields non-nil
- [ ] edge: injection text that is borderline (cos ≈ τ ± 0.01) — with real
      model on aiwonder, print the exact cos and τ; assert consistent with
      the `pass` flag in the verdict

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root
  `/home/croyse/calyx/data/fsv-issue273-ph39-t05-<date>/` containing the
  captured cargo log, failing style-slot verdict JSON, quarantine
  `NoveltyRecord` readback JSON, accepted in-persona verdict JSON, and
  SHA-256 manifest. Stdout and in-memory state are claims; the durable JSON
  readback files are the verdict.
- **Readback:**
  ```
  root=/home/croyse/calyx/data/fsv-issue273-ph39-t05-<date>
  mkdir -p "$root"
  cargo test -p calyx-ward fsv_injection -- --nocapture 2>&1 | tee "$root/ph39-injection-fsv.log"
  grep -E "Quarantined|style|cos|tau|pass|guarded:pass" "$root/ph39-injection-fsv.log"
  xxd -g 1 "$root/quarantine-record-readback.json" | head -32
  xxd -g 1 "$root/in-persona-accepted-readback.json" | head -32
  sha256sum "$root"/* | sort
  ```
- **Prove:** `Quarantined` appears with a `style` slot where `pass: false`;
  `cos` value < `tau` value printed for the injection case; `guarded:pass`
  appears for the in-persona case; `NoveltyRecord` JSON shows valid UUID;
  attach the root path, hashes, and durable JSON readback excerpts to the PH39
  GitHub issue

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH39 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
