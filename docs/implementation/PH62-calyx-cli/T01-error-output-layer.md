# PH62 · T01 — Error wiring and output layer

| Field | Value |
|---|---|
| **Phase** | PH62 — calyx-cli (vault/lens/ingest/search/readback) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/error.rs` (≤500), `crates/calyx-cli/src/output.rs` (≤500), `crates/calyx-cli/src/main.rs` (extend, ≤500) |
| **Depends on** | — (calyx-core error catalog already exists) |
| **Axioms** | A16, A17 |
| **PRD** | `dbprdplans/18 §6`, `dbprdplans/14 §1` |

## Goal

Establish the CLI's error-handling and output contracts before any subcommand is
built. Every `CalyxError` serializes to `{"code":"CALYX_*","message":"…",
"remediation":"…"}` on stderr (exit 2). Output helpers emit JSON, human-readable
tables, and hex-dump rows so every downstream subcommand uses one canonical path.
The primary consumer is an AI agent (A17) that self-corrects from `code` +
`remediation`.

## Build (checklist of concrete, code-level steps)

- [ ] `crates/calyx-cli/src/error.rs`: `CliError` enum wrapping `CalyxError` (from
  calyx-core) + `IoError(String)` + `UsageError(String)`; `impl From<CalyxError>`
  and `impl From<io::Error>`; `fn to_json(&self) -> String` serializes to
  `{"code","message","remediation"}` — `IoError`/`UsageError` use sentinel codes
  `CALYX_CLI_IO_ERROR` / `CALYX_CLI_USAGE_ERROR` with explicit remediation strings
- [ ] `crates/calyx-cli/src/output.rs`: `fn print_json<T: Serialize>(val: &T)`,
  `fn print_table(headers: &[&str], rows: &[Vec<String>])`, `fn print_hex_dump(
  offset: u64, bytes: &[u8])` (16 bytes/row, offset + hex + ASCII); all write to
  stdout
- [ ] Extend `main.rs`: replace bare `eprintln!` + `ExitCode::from(2)` with
  `CliError::emit(&self)` that writes `to_json()` to stderr then exits 2; preserve
  existing `readback --hex` / `--vault-tree` paths unchanged
- [ ] `print_hex_dump` format: `{offset:08x}  {hex pairs space-separated}  |{ascii}|`
  matching `xxd -g 1` layout so FSV readers can cross-verify

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `CliError::from(CalyxError::lens_dim_mismatch("got 384, expected 768"))
  .to_json()` equals exact JSON string `{"code":"CALYX_LENS_DIM_MISMATCH",
  "message":"got 384, expected 768","remediation":"fix lens or slot shape"}`
- [ ] unit: `CliError::UsageError("bad arg".into()).to_json()` contains
  `"code":"CALYX_CLI_USAGE_ERROR"` and non-empty `"remediation"`
- [ ] proptest: for any `CalyxErrorCode`, round-trip through `CliError` preserves
  `.code` and `.remediation` verbatim
- [ ] unit: `print_hex_dump(0, b"\x00\x41\x42\x43")` first line contains
  `"00000000"` and `"|.ABC|"`
- [ ] edge: zero-length byte slice → `print_hex_dump` emits nothing; 17-byte input
  → two rows; all-0xff input → `"ff"` hex pairs + `"."` ASCII
- [ ] fail-closed: `CliError::emit` calls `process::exit(2)` — verified by
  spawning a subprocess and asserting exit code 2

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** stderr output of a `calyx` invocation with a deliberately invalid
  argument
- **Readback:** `calyx readback --hex /nonexistent 2>&1; echo "exit:$?"` — stderr
  carries the JSON error, stdout is empty, exit code is 2
- **Prove:** the raw stderr bytes contain `CALYX_CLI_IO_ERROR` (or the appropriate
  `CALYX_*` code), a non-empty `message`, and a non-empty `remediation`; exit = 2;
  nothing is written to stdout

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH62 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
