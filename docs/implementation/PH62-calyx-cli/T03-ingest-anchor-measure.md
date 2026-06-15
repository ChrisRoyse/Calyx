# PH62 · T03 — Ingest, anchor, and measure subcommands

| Field | Value |
|---|---|
| **Phase** | PH62 — calyx-cli (vault/lens/ingest/search/readback) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/cmd/ingest.rs` (≤500) |
| **Depends on** | T02 (subcommand dispatch), PH09 (constellation CRUD + idempotent ingest) |
| **Axioms** | A1, A15, A16, A17 |
| **PRD** | `dbprdplans/14 §2` (ingest/measure group), `dbprdplans/18 §4` |

## Goal

Wire the `ingest`, `anchor`, and `measure` subcommands to the engine API.
`ingest` is content-addressed and idempotent: re-ingesting the same bytes returns
the same `CxId` without creating a duplicate constellation (A1). Every ingest
writes a Ledger entry (A15 stub until PH35). `measure` embeds without storing —
used to guard a candidate. `anchor` attaches a grounded outcome to a known `CxId`.

## Build (checklist of concrete, code-level steps)

- [ ] `cmd/ingest.rs` — `ingest <vault> --text <s> [--batch <jsonl-path>]
  [--idempotent (default: true)]`: calls `Calyx::ingest(vault, Input::Text(s))`
  or `Calyx::ingest_batch(vault, inputs)` for batch path; prints
  `{"cx_id":"<hex16>","new":true|false,"ledger_seq":<u64>}` per item; `--batch`
  reads a JSONL file of `{"text":"…"}` objects, one constellation per line
- [ ] Idempotent contract: second `ingest` of same text must return `"new":false`
  and the same `cx_id` — verified by calling the engine's content-addressed CxId
  derivation (blake3(input ‖ panel_ver ‖ salt))
- [ ] `cmd/ingest.rs` — `anchor <vault> <cx_id> --kind <kind> --value <v>
  [--confidence <f32>] [--source <s>]`: parses `AnchorKind` (`test-pass`,
  `thumbs-up`, `thumbs-down`, `label:<str>`, `speaker-match`, `style-hold`);
  calls `Calyx::anchor(vault, cx_id, Anchor{…})`; prints
  `{"status":"anchored","cx_id":"…","ledger_seq":<u64>}`
- [ ] `cmd/ingest.rs` — `measure <vault> --text <s>`: calls `Calyx::measure(vault,
  input)` (no store); prints constellation JSON with all slot vectors present
  (for guard inspection); `SlotVector::Absent` slots print `{"absent":{"reason":"…"}}`
  — never zero-filled (A16)
- [ ] Error paths: missing vault → `CALYX_VAULT_ACCESS_DENIED`; lens runtime
  unreachable during ingest → `CALYX_LENS_UNREACHABLE`, remediation instructs
  `restore lens service`; NaN/Inf vector output → `CALYX_LENS_NUMERICAL_INVARIANT`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: ingest text "hello" twice into a seeded test vault → both calls return
  the same `cx_id`; second call returns `"new":false`
- [ ] unit: `anchor` with `--kind label:positive` → parsed `AnchorKind::Label(
  "positive")`; serialized Anchor round-trips
- [ ] unit: `measure` on known input → `SlotVector::Absent` slots present in
  output JSON (not zero-filled); `SlotVector::Dense(vec)` has finite values
- [ ] proptest: for any valid text input, `CxId` derivation is deterministic
  (same text + same panel_ver + same salt → same bytes)
- [ ] edge: batch JSONL with 0 lines → prints nothing, exits 0; JSONL with a line
  that is not valid JSON → `CALYX_CLI_IO_ERROR` mid-batch with line number; empty
  `--text ""` → `CALYX_CLI_USAGE_ERROR`
- [ ] fail-closed: `anchor` with unknown `cx_id` → `CALYX_VAULT_ACCESS_DENIED` on
  stderr, exit 2; `anchor` with `--confidence 1.5` (out of range) →
  `CALYX_CLI_USAGE_ERROR`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the base CF row for the newly ingested constellation at
  `<vault.calyx>/cf/base/<cx_id_hex>`
- **Readback:** `calyx readback --hex <vault.calyx>/cf/base/<cx_id_hex>` after
  `calyx ingest` → prints the raw bytes of the constellation record; compare
  `cx_id` embedded in the bytes against the CxId printed by `ingest`
- **Prove:** bytes are present and non-empty after first ingest; re-running
  `ingest` with identical input leaves the bytes unchanged (idempotent: same
  file, same length); `anchor` adds a new row in `cf/anchors/<cx_id_hex>` — read
  with `calyx readback --hex <vault.calyx>/cf/anchors/<cx_id_hex>` to confirm

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH62 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
