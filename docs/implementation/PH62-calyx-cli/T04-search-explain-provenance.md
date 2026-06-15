# PH62 · T04 — Search with explain and provenance

| Field | Value |
|---|---|
| **Phase** | PH62 — calyx-cli (vault/lens/ingest/search/readback) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/cmd/search.rs` (≤500) |
| **Depends on** | T03 (ingest exists), PH24 (RRF fusion + provenance hits), PH26 (query planner + explain) |
| **Axioms** | A17, A15 |
| **PRD** | `dbprdplans/14 §2` (search/navigate group), `dbprdplans/18 §5` |

## Goal

Implement `calyx search` — the everyday multi-lens search command. RRF fusion is
the default; results include per-lens contributions when `--explain` is set;
every hit carries a `LedgerRef` (provenance by default, A15). The agent should
get a working search with one argument and opt into deeper inspection via flags.
Also implements `kernel-answer` for grounded kernel-skeleton responses.

## Build (checklist of concrete, code-level steps)

- [ ] `cmd/search.rs` — `search <vault> <query> [--k <n=10>] [--fusion rrf|
  weighted-rrf|single-lens|kernel-first|pipeline] [--guard off|in-region]
  [--explain] [--provenance (default: true)] [--fresh|--stale-ok]
  [--filter <json-predicate>]`: builds `Query{input: QueryInput::Text(query),
  lenses: LensSel::Auto, fusion: Fusion::Rrf, k, guard, explain, …}` and
  calls `Calyx::search(vault, query)`
- [ ] Output format for each `Hit`:
  ```json
  {"rank":1,"cx_id":"…","score":0.834,
   "per_lens":[{"slot":0,"rank":2,"raw":0.91,"weight":0.5,"contribution":0.455}],
   "guard":{"verdict":"pass","tau":0.12},
   "provenance":{"ledger_seq":42,"chain_hash":"…"}}
  ```
  `per_lens` present only when `--explain`; `guard` present only when guard mode
  is not `Off`; `provenance` always present (A15)
- [ ] `cmd/search.rs` — `kernel-answer <vault> <query> [--anchor <kind>]
  [--explain]`: calls `Calyx::kernel_answer(vault, q, anchor)` → prints
  `{"answer":"…","kernel_cx_ids":["…"],"recall":0.97,"gaps":["…"]}`;
  `CALYX_KERNEL_UNGROUNDED` when kernel is not grounded → remediation
  `"add anchors (grounding_gaps)"` printed on stderr
- [ ] `--provenance` default true: every search result includes `LedgerRef`
  without the user specifying a flag; `--no-provenance` disables for bulk/debug
- [ ] All `CALYX_*` errors route through `CliError` → JSON stderr + exit 2;
  notably `CALYX_GUARD_OOD` when a result is blocked, `CALYX_STALE_DERIVED`
  when freshness cannot be satisfied

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `Query` built from `["search", "myvault", "hello", "--k", "5"]` has
  `k=5`, `fusion=Rrf`, `explain=false`, `guard=Off`, provenance enabled
- [ ] unit: `["search", "myvault", "hello", "--explain"]` sets `explain=true` and
  result JSON contains `per_lens` array with at least one entry
- [ ] unit: `kernel-answer` with ungrounded vault → exit code 2, stderr contains
  `"CALYX_KERNEL_UNGROUNDED"` and `"add anchors"` in `remediation`
- [ ] proptest: for any `Hit` struct, serializing then round-tripping through the
  output formatter preserves `cx_id` bytes exactly (hex encoding stable)
- [ ] edge: `--k 0` → `CALYX_CLI_USAGE_ERROR`; `--fusion unknown-mode` →
  `CALYX_CLI_USAGE_ERROR`; vault with 0 constellations → empty results array `[]`,
  exit 0 (not an error)
- [ ] fail-closed: `search` when guard blocks all results → returns `[]` with
  `CALYX_GUARD_OOD` in stderr per blocked item; never silently returns 0-score
  results as if unguarded

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the `LedgerRef.ledger_seq` returned in the search hit's provenance
  field; the corresponding Ledger entry on disk
- **Readback:** after running `calyx search aiwonder-test "fail under load"
  --explain --provenance`, extract `ledger_seq` from the JSON output and run
  `calyx readback --ledger <vault.calyx>/ledger/<seq_hex>` to read the raw
  Ledger entry bytes; also `xxd <vault.calyx>/ledger/<seq_hex>` for cross-check
- **Prove:** the Ledger entry bytes exist, are non-empty, and contain the `CxId`
  of the top search hit (bytes match); `per_lens` in the explain output lists
  all active slots with non-zero contributions; provenance is present in every
  hit even without `--explain`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH62 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
