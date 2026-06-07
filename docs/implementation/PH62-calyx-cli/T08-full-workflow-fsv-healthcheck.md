# PH62 · T08 — Full workflow FSV and healthcheck

| Field | Value |
|---|---|
| **Phase** | PH62 — calyx-cli (vault/lens/ingest/search/readback) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/cmd/healthcheck.rs` (≤500) |
| **Depends on** | T02, T03, T04, T05, T06, T07 (all prior commands exist) |
| **Axioms** | A16, A17 |
| **PRD** | `dbprdplans/14 §3`, `dbprdplans/25_STAGE15_INTERFACES.md` (FSV gate) |

## Goal

Implement `calyx healthcheck` and run the full end-to-end workflow FSV proof on
aiwonder: `create-vault → add-lens → ingest → anchor → search → readback`. This
card is the phase exit gate: every command from T02–T07 must chain together,
produce real bytes on disk, and the readback must match a direct file read.
`healthcheck` probes the engine and lens runtimes and returns a structured
pass/fail. This is the last card; its FSV evidence closes the PH62 GitHub issue.

## Build (checklist of concrete, code-level steps)

- [ ] `cmd/healthcheck.rs` — `healthcheck [--vault <vault>] [--json]`:
  probes in sequence: (1) `Calyx::resource_status` (engine live); (2) TEI
  endpoints `:8088/:8089/:8090` reachable (HTTP GET with 2s timeout); (3) if
  `--vault` given, runs `Calyx::abundance(vault)` to confirm the vault is
  accessible and returns at least one CF entry; prints:
  ```json
  {"status":"pass","checks":[
    {"name":"engine","status":"pass"},
    {"name":"tei:8088","status":"pass","latency_ms":3},
    {"name":"tei:8089","status":"pass","latency_ms":4},
    {"name":"tei:8090","status":"pass","latency_ms":4},
    {"name":"vault","status":"pass","n_cx":142}
  ]}
  ```
  any failing check → `{"status":"fail",…}` with `CALYX_LENS_UNREACHABLE` or
  `CALYX_FORGE_DEVICE_UNAVAILABLE` for the failing component, exit 2
- [ ] `--json` flag (default: true for machine consumers, A17); human mode
  (`--no-json`) prints `PASS` / `FAIL <component>` one per line
- [ ] Full workflow integration test harness (used for FSV, not as the FSV
  itself): a `#[test]` that runs `create-vault → add-lens → ingest → anchor →
  search → readback --cf-row` against a temp-dir vault and asserts the final
  `readback` bytes are non-empty and contain the `cx_id` hex
- [ ] Idempotency regression: run the full workflow twice; second run's ingest
  returns `"new":false` for all items; readback bytes are unchanged

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] integration: `create-vault + add-lens + ingest + anchor + search` on a
  seeded temp vault → search returns the ingested `cx_id` in top-k results;
  `readback --cf-row` on that `cx_id` returns non-empty bytes
- [ ] unit: `healthcheck` with all probes passing → `{"status":"pass"}`, exit 0
- [ ] unit: `healthcheck` with TEI at `:8088` unreachable → `{"status":"fail",
  "checks":[…{"name":"tei:8088","status":"fail"}…]}`, exit 2,
  `CALYX_LENS_UNREACHABLE` on stderr
- [ ] idempotency: ingest same 5 texts twice → second batch: all `"new":false`,
  vault `n_cx` unchanged, readback bytes identical to first run
- [ ] edge: `healthcheck` on a corrupted vault (`manifest/CURRENT` deleted) →
  `CALYX_ASTER_CORRUPT_SHARD` in the vault check entry, exit 2; never hangs
  waiting on an unreachable TEI (timeout fires within 2s)
- [ ] fail-closed: `healthcheck` when CUDA unavailable (server mode) →
  `CALYX_FORGE_DEVICE_UNAVAILABLE` in checks; CPU fallback probe still passes
  if CPU SIMD Forge is available

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the full aiwonder vault at `$CALYX_HOME/vaults/aiwonder-test/` after
  running the complete workflow sequence
- **Readback:** execute the following on aiwonder (not in CI, not in a harness):
  ```
  calyx create-vault aiwonder-test --panel-template text-default
  calyx add-lens aiwonder-test --name gte-768 --runtime tei-http \
      --endpoint http://localhost:8088 --shape Dense\(768\)
  calyx ingest aiwonder-test --text "Why does X fail under load?"
  # capture cx_id from output
  calyx anchor aiwonder-test <cx_id> --kind test-pass --value true
  calyx search aiwonder-test "fail under load" --explain --provenance
  calyx readback --cf-row $CALYX_HOME/vaults/aiwonder-test \
      --cf base --key <cx_id_hex>
  xxd $CALYX_HOME/vaults/aiwonder-test/cf/base/<cx_id_hex>
  ```
  The `readback --cf-row` output and `xxd` output must match byte-for-byte.
  `calyx healthcheck --vault aiwonder-test` must return `{"status":"pass"}`.
- **Prove:** readback bytes are present, non-empty, and byte-identical to `xxd`;
  search result includes the ingested `cx_id` in top-k; provenance field contains
  a non-zero `ledger_seq`; `healthcheck` exits 0

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH62 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
