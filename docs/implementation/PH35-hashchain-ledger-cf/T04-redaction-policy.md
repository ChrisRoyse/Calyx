# PH35 · T04 — Redaction policy: no secrets in payload

| Field | Value |
|---|---|
| **Phase** | PH35 — Hash-chain append-only CF (in group-commit) |
| **Stage** | S7 — Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/redaction.rs` (≤500) |
| **Depends on** | T03 (this phase) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 §4`, `11 §2` |

## Goal

Enforce the PRD rule that Ledger payloads store hashes and IDs only — never
raw secret values, bearer tokens, or sensitive text. Provenance holds via
content-addressed hashes; privacy holds because the actual bytes are never
written. This is the `input_hash always stored; raw input bytes optional`
contract from `11 §4`.

## Build (checklist of concrete, code-level steps)

- [ ] `struct RedactionPolicy` — configurable per vault: `store_raw_input: bool`
  (default `false`); `redact_actor_name: bool` (default `false`).
- [ ] `fn RedactionPolicy::check_payload(payload: &[u8]) -> Result<()>` —
  scans the payload for heuristic secret patterns (high-entropy token-like
  strings of length ≥ 40 with no spaces, or fields named `password`/`token`/
  `secret`/`key` in the serialised JSON); returns
  `CALYX_LEDGER_SECRET_IN_PAYLOAD` if found.
  Add `CALYX_LEDGER_SECRET_IN_PAYLOAD` to `calyx-core/src/error.rs` with
  remediation `"ledger payload must store hashes/ids only — redact before writing"`.
- [ ] `fn RedactionPolicy::redact_input_ref(input_ref: &InputRef) -> RedactedInput` —
  always emits `{ hash: input_ref.hash, redacted: true }`; never copies the
  `pointer` field into the ledger payload (the pointer may contain a path that
  leaks structure).
- [ ] `fn RedactionPolicy::apply_to_payload(raw: &PayloadBuilder) -> Vec<u8>` —
  strips any `raw_bytes` fields; retains only `cx_id`, `lens_id`,
  `weights_sha256`, `input_hash`, `ts`, and similar ID/hash fields.
- [ ] `LedgerAppender::append` calls `check_payload` before encoding; on
  failure returns `CALYX_LEDGER_SECRET_IN_PAYLOAD`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: payload = JSON `{"input_hash":"aabb...","cx_id":"..."}` → `check_payload`
  returns `Ok(())`; assert passes.
- [ ] unit: payload = JSON `{"password":"hunter2"}` → `check_payload` returns
  `Err(CALYX_LEDGER_SECRET_IN_PAYLOAD)`.
- [ ] unit: payload = 44-char base64-like string with no spaces →
  `check_payload` returns `Err(CALYX_LEDGER_SECRET_IN_PAYLOAD)`.
- [ ] edge (≥3): empty payload → `Ok(())`; payload with `input_hash` that is
  exactly 64 hex chars (valid hash) → `Ok(())`; payload with a 40-char random
  printable ASCII run → `Err(CALYX_LEDGER_SECRET_IN_PAYLOAD)`.
- [ ] fail-closed: `redact_input_ref` with non-empty `pointer` → returned
  `RedactedInput.pointer` is `None`; assert no pointer bytes appear in output.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `ledger` CF row payloads after writing a constellation ingest entry
- **Readback:** `calyx scan --cf ledger --seq <n> | jq '.payload'` — confirm
  the payload JSON contains only `cx_id`, `input_hash`, `lens_id`, and
  similar hash/ID fields; confirm no field name matches `password`, `token`,
  `secret`, `key`, or contains a raw string longer than 64 characters that
  isn't a hex/base58/UUID.
- **Prove:** before: no redaction guard; after: scan of ledger CF shows no
  secret-like fields in any payload; `check_payload` test with a bearer-token
  string returns `CALYX_LEDGER_SECRET_IN_PAYLOAD` (not a panic, not silent).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH35 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
