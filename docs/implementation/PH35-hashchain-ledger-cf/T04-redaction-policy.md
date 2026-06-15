# PH35 - T04 - Redaction policy: no secrets in payload

| Field | Value |
|---|---|
| **Phase** | PH35 - Hash-chain append-only CF (in group-commit) |
| **Stage** | S7 - Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/redaction.rs` (<=500) |
| **Depends on** | T03 (this phase) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 §4`, `11 §2` |

## Goal

Enforce the PRD rule that Ledger payloads store hashes and IDs only, never raw
secret values, bearer tokens, or sensitive text. Provenance holds through
content-addressed hashes; privacy holds because raw bytes are redacted before
append.

## Build

- [x] `struct RedactionPolicy` configurable per vault:
  `store_raw_input: bool` defaults to `false`; `redact_actor_name: bool`
  defaults to `false`.
- [x] `RedactionPolicy::check_payload(payload: &[u8]) -> Result<()>` scans
  payloads for secret-like JSON fields (`password`, `token`, `secret`, `key`)
  and high-entropy token-like strings of length >=40.
- [x] `CALYX_LEDGER_SECRET_IN_PAYLOAD` exists in `calyx-core/src/error.rs`
  with remediation: `ledger payload must store hashes/ids only - redact before writing`.
- [x] `RedactionPolicy::redact_input_ref(input_ref: &InputRef) ->
  RedactedInput` emits `{ hash, redacted: true }` and never copies `pointer`.
- [x] `RedactionPolicy::apply_to_payload(raw: &PayloadBuilder) -> Vec<u8>`
  strips `raw_bytes` and secret fields, retaining hashes, IDs, `ts`, and
  redaction markers.
- [x] `LedgerAppender::append` checks payloads before encoding or writing a
  ledger row; failures return `CALYX_LEDGER_SECRET_IN_PAYLOAD`.

## Tests

- [x] Safe JSON with `input_hash`, `cx_id`, and `lens_id` is accepted.
- [x] JSON `{"password":"hunter2"}` is rejected.
- [x] A 44-character base64-like token with no spaces is rejected.
- [x] Edges: empty payload accepted; 64-hex `input_hash` accepted; 40-character
  printable ASCII secret run rejected.
- [x] `redact_input_ref` omits a non-empty pointer and serialized output
  contains no pointer bytes.
- [x] `LedgerAppender` rejects a secret payload without writing any row.

## FSV

- **SoT:** physical disk-backed ledger CF row at
  `/home/croyse/calyx/data/fsv-issue245-ledger-redaction-20260608/redaction-policy/ledger-cf/0000000000000000.ledger`
- **Trigger:** `cargo test -p calyx-ledger --test appender_fsv
  ph35_ledger_redaction_aiwonder_fsv -- --ignored --nocapture` on aiwonder.
- **Readback:** decoded payload JSON plus direct `strings` scan of the `.ledger`
  row. The stored row contains only `cx_id`, `input_hash`, `input_ref.hash`,
  `input_ref.redacted`, `lens_id`, `ts`, and `weights_sha256`; forbidden
  markers `password`, `token`, `secret`, `hunter2`, `Bearer`, `raw_bytes`, and
  the synthetic pointer path are absent from the row bytes.

Evidence root:
`/home/croyse/calyx/data/fsv-issue245-ledger-redaction-20260608`

Evidence hashes:

- `issue245-gates-and-fsv.log`:
  `66d89a2e51b87663a07ec51e4ff2114022cdb12ce0084e9914925bdf6bbfda4b`
- `issue245-source-readback.log`:
  `e9c9e8cca65d55a840f2194cc8c32cdbc2cd52cd43a05cb9e26eb531c991e1c7`
- `redaction-policy/ledger-redaction-readback.json`:
  `322de68461d871913b4210e4a2fc82dca6f1712466cfcfb0a7bff2543b2a99f9`
- `redaction-policy/ledger-cf/0000000000000000.ledger`:
  `2a8fc4027d181164769ade529b4f14caafd0296f18f71f728870b6134b91c3d5`

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder.
- [x] `.rs` files remain <=500 lines.
- [x] FSV evidence attached to GitHub issue #245.
- [x] No anti-pattern: no flattening, no trusted ungrounded claim, no frozen-lens
  mutation, and no harness-only verdict.
