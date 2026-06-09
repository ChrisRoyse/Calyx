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
readable from the durable aiwonder novelty/vault evidence, confirming the
routing at the source of truth.

## Status

DONE / FSV-signed-off at implementation commit `8d2572b`. Durable aiwonder
evidence:
`/home/croyse/calyx/data/fsv-issue273-ph39-t05-20260609-8d2572b-ort126-sm120`.

Readback summary:
- Source injection row: `deepset/prompt-injections` `train-4`, label `1`,
  text SHA-256 `e4822f0765ce9901257d930a28b1acfe50ead8735a026129b50c42c5397feaea`.
- Style model SHA-256:
  `fc3c80ead2e4ceef693fa67756f2e0f920fee7df326a565286b34d68d7a170af`.
- Matched style dim 768; norm `1.0000007152557373`.
- Injection output: `Quarantined`, action `Quarantine`, numeric style slot `9`,
  cos `0.5983942747116089` < tau `0.9900000095367432`, pass `false`.
- In-persona output: `guarded:pass`, overall pass `true`, style slot cos `1.0`.
- Full manifest SHA-256:
  `c4f7d4a9d3ffcab7650c0432ba58075b068433158568dfd8ee1191de00208329`.

## Build (checklist of concrete, code-level steps)

- [x] Write `#[test] fn issue273_identity_injection_quarantine_fsv_writes_readbacks`:
      - Load the style `IdentityProfile` with calibrated τ on the style slot
        (from `/home/croyse/calyx/data/identity_fsv/style_profile.json` on
        aiwonder; absence is setup failure, not a passing skip)
      - Load in-persona text from
        `/home/croyse/calyx/data/identity_fsv/in_persona_01.txt`, embed it with
        the pinned `StyleLens`, and write `matched-style-readback.json` into the
        evidence root. Ward has no NPY reader in this crate; do not claim
        `matched_style.npy` unless a reader or documented conversion is added.
      - Load one real injection text copied from the on-disk
        `/home/croyse/calyx/data/injection_corpus/raw.jsonl` corpus into
        `/home/croyse/calyx/data/identity_fsv/injection_01.txt`, with the source
        row and SHA recorded in `injection_source_*.json`
      - Use `StyleLens` (real model on aiwonder; mock on dev)
      - Call `guard_generate()` with `novelty_action: Quarantine`,
        `high_stakes: false`
      - Assert `GenerateOutput::Novel { record }` where
        `record.status == Quarantined` and `record.action_taken == Quarantine`
      - Print `record.failing_verdicts` — show per-slot `(cos, tau, pass)` on
        the style slot
      - Assert `record.failing_verdicts.iter().any(|v| v.slot == style_slot_id && !v.pass)`;
        Calyx `SlotId` values are numeric, so `style_slot_id` must come from
        the configured identity profile/panel mapping, not a string literal.
- [x] Write in-persona accepted path in the same manual FSV fixture:
      - Load an in-persona text sample from
        `/home/croyse/calyx/data/identity_fsv/in_persona_01.txt`
      - Same profile and matched vecs as above
      - Call `guard_generate()`
      - Assert `GenerateOutput::Accepted { provenance_tag: "guarded:pass" }`
      - Print per-slot verdicts; assert all `pass == true`
- [x] Write durable quarantine-record sink readback in the same manual FSV fixture:
      - Confirm `NoveltyRecord` is written to the `VaultSink`; read
        `vault.novel_records()` or a dedicated quarantine readback from the
        durable sink used by the aiwonder fixture. Do not use
        `novel_regions(since=0)` for quarantine proof: that API only returns
        `AwaitingGrounding` records, not `Quarantined` records.
      - Assert record present with `status: Quarantined`; print as JSON
      - `novel_id` is a non-nil UUID; `guard_id` matches the profile
- [x] Missing aiwonder data files fail the manual fixture with a clear setup
      error. Non-aiwonder dev coverage may use ignored tests/mocks, but it must
      not produce a successful FSV artifact.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: injection quarantine fixture asserts `Quarantined`;
      prints failing style-slot verdict
- [x] unit: in-persona fixture asserts `Accepted`;
      prints `"guarded:pass"` provenance tag
- [x] unit: quarantine record readable; all fields non-nil
- [x] edge: injection text that is borderline/nearer-persona — with real model
      on aiwonder, print the exact cos and tau; assert consistent with the
      `pass` flag in the verdict

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root
  `/home/croyse/calyx/data/fsv-issue273-ph39-t05-20260609-8d2572b-ort126-sm120/`
  containing the
  captured cargo log, failing style-slot verdict JSON, quarantine
  `NoveltyRecord` readback JSON, accepted in-persona verdict JSON, and
  SHA-256 manifest. Stdout and in-memory state are claims; the durable JSON
  readback files are the verdict.
- **Readback:**
  ```
  root=/home/croyse/calyx/data/fsv-issue273-ph39-t05-20260609-8d2572b-ort126-sm120
  CALYX_WARD_IDENTITY_FSV_DIR="$root" cargo test -p calyx-ward --test identity_fsv issue273_identity_injection_quarantine_fsv_writes_readbacks -- --ignored --nocapture
  xxd -g 1 "$root/quarantine-record-readback.json" | head -32
  xxd -g 1 "$root/in-persona-accepted-readback.json" | head -32
  (cd "$root" && sha256sum -c SHA256SUMS.txt && sha256sum -c SHA256SUMS.full.txt)
  ```
- **Prove:** `Quarantined` appears with a `style` slot where `pass: false`;
  `cos` value < `tau` value printed for the injection case; `guarded:pass`
  appears for the in-persona case; `NoveltyRecord` JSON shows valid UUID;
  attach the root path, hashes, and durable JSON readback excerpts to the PH39
  GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH39 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
