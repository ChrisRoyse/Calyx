# PH72 · T06 — Universal summarization via multi-scope kernel (`summarize(scope)`)

| Field | Value |
|---|---|
| **Phase** | PH72 — Streaming + Reactive + Time-Travel + Universal Summarization |
| **Stage** | S20 — Critical Capabilities |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/summarize.rs` (≤500) |
| **Depends on** | PH34 (`build_kernel` + `Scope` enum + `ScopeCache`), PH33 (`kernel_answer` + `grounding_gaps`), PH35 (Ledger CF) |
| **Axioms** | A21, A24, A15, A11 |
| **PRD** | `17 §8`, `08 §4b` |

## Goal

Expose a single `summarize(vault, scope, params?) -> Result<SummarizeResult, CalyxError>`
function that makes the multi-scope kernel the universal summarization primitive:
"the core of ANY slice." For any `Scope` (collection, domain, time window, tenant,
union, etc.) it builds or retrieves the cached kernel, measures `kernel_only_recall`
and `grounded_fraction`, and returns the kernel node IDs with their metrics.
This is structural summarization only — strict Royse theory (A24): the summary IS
the kernel nodes, not generated text. Each call is Ledger-provenanced (A15) with
a `SUMMARIZE_INVOKED` entry containing the scope hash and result metrics.

## Build (checklist of concrete, code-level steps)

- [ ] `SummarizeParams { max_kernel_size: Option<usize>, require_grounded: bool, cache_ttl_secs: Option<u64> }` — all optional; default `max_kernel_size: None`, `require_grounded: false`, `cache_ttl_secs: Some(3600)`
- [ ] `SummarizeResult { scope_hash: [u8;32], kernel_ids: Vec<CxId>, kernel_size: usize, kernel_only_recall: f32, grounded_fraction: f32, approx_factor: f32, ledger_ref: LedgerRef }` — `approx_factor` from hierarchical kernel if used; `ledger_ref` points to the `SUMMARIZE_INVOKED` entry
- [ ] `fn summarize(vault: &Vault, scope: Scope, params: Option<SummarizeParams>, clock: &dyn Clock) -> Result<SummarizeResult, CalyxError>`:
  - compute `scope_hash = scope_hash(scope)` (from PH34)
  - call `build_kernel(vault, scope, None, params.max_kernel_size)` — delegates through `ScopeCache` (cache hit or miss)
  - if `params.require_grounded && kernel.grounded_fraction < 0.5` → return `CALYX_SUMMARIZE_INSUFFICIENT_GROUNDING` with `grounded_fraction` in error metadata (fail-closed, not a silent partial result)
  - write `SUMMARIZE_INVOKED { scope_hash, kernel_size, kernel_only_recall, grounded_fraction }` Ledger entry (A15)
  - return `SummarizeResult` with all fields populated
- [ ] `fn summarize_as_of(vault: &Vault, scope: Scope, t: Timestamp, params: Option<SummarizeParams>, clock: &dyn Clock) -> Result<SummarizeResult, CalyxError>` — calls `as_of(vault, t, clock)` to get a `TimeTravelSnapshot`, then calls `summarize` with the snapshot as the data source; if `t` is before the retention horizon → propagates `CALYX_TIMETRAVEL_BEFORE_HORIZON` unchanged (no data returned)
- [ ] Export `summarize` and `summarize_as_of` from `calyx-lodestar/src/lib.rs`
- [ ] `SummarizeResult` implements `Display` that prints a table: `scope_hash | kernel_size | recall | grounded_fraction | approx_factor` — used by CLI readback

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: build a 20-node synthetic corpus; `summarize(Scope::AllAssociations, None)` returns `kernel_ids` that are a strict subset of all 20 CxIds; `kernel_only_recall ≥ 0.0` and `≤ 1.0`; `kernel_size ≤ 20`; Ledger entry present (read `vault.ledger_tail(1)` → kind == `SUMMARIZE_INVOKED`)
- [ ] unit: `summarize(Scope::Collection(coll_id), None)` on a collection with 5 planted bridge nodes → `kernel_ids` contains only CxIds from that collection; other collection's CxIds absent
- [ ] unit: `summarize(scope, Some(SummarizeParams { require_grounded: true, .. }))` on a corpus where `grounded_fraction < 0.5` → `CALYX_SUMMARIZE_INSUFFICIENT_GROUNDING` error; `kernel_ids` NOT present in the error (fail-closed, A16)
- [ ] unit: `summarize_as_of(scope, t=past)` with a vault that has a retention horizon after `t` → `CALYX_TIMETRAVEL_BEFORE_HORIZON`; no kernel result returned
- [ ] proptest: `∀ scope ∈ [AllAssociations, Collection(id), TimeWindow(t0,t1)]` on the same corpus: `summarize` returns `kernel_only_recall ∈ [0.0, 1.0]`; `grounded_fraction ∈ [0.0, 1.0]`; `kernel_size ≥ 0`; no panic
- [ ] edge: `summarize` on an empty vault (0 constellations) → `kernel_ids = []`, `kernel_size = 0`, `kernel_only_recall = 0.0`; Ledger entry still written; no panic
- [ ] edge: `summarize` with `cache_ttl_secs: Some(0)` (no cache) → two successive calls both re-compute; different `ledger_ref` for each; kernel result consistent
- [ ] fail-closed: `Scope::TimeWindow` with `t0 > t1` (inverted window) → `CALYX_SCOPE_INVALID_TIME_WINDOW`; no kernel built

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the `SummarizeResult` JSON written to `$CALYX_HOME/fsv/ph72_summarize_*.json`; the Ledger `SUMMARIZE_INVOKED` entry; the `kernel_ids` list readable back from the vault
- **Readback:** `calyx summarize --vault $VAULT_PATH --scope collection:<COLL_ID> --out $CALYX_HOME/fsv/ph72_summarize_01.json` → writes the result; `cat $CALYX_HOME/fsv/ph72_summarize_01.json | jq '{kernel_size, kernel_only_recall, grounded_fraction}'` → non-empty values; `calyx readback ledger-tail --vault $VAULT_PATH --n 1` → kind `SUMMARIZE_INVOKED`, `scope_hash` matches
- **Prove:** run `summarize` on a real corpus (e.g. the Leapable vault or aiwonder dataset); the JSON output contains ≥1 `kernel_id`; `kernel_only_recall` is a finite non-zero float (not 0.0 on a non-trivial corpus); Ledger entry byte-readable; run `summarize_as_of` at `t` before ingestion of a batch → `kernel_size` is smaller than post-batch run (historical summary differs from present); both JSON files present and machine-readable

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH72 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
## Issue #757 follow-up implementation

#757 completes the three deliberate gaps left after #576:

- `summarize_with_recall` accepts optional recall inputs and measures
  `kernel_only_recall` through the existing Lodestar `kernel_recall_test`
  path after the kernel is built. With per-node embeddings present, the result
  is a finite measured value instead of the default `0.0`.
- `AsterAssocSnapshot` is the production `AsterVault -> AssocStore` bridge.
  It reads `cf/graph` `PlainGraph` rows, JSON node props, and the
  `lodestar_assoc_v1` graph metadata row. That metadata carries the
  vault-embedded `retention_horizon`; `summarize_vault_as_of` fails closed with
  `CALYX_TIMETRAVEL_BEFORE_HORIZON` before opening a historical snapshot when
  `t` is outside the retained horizon.
- `calyx summarize --vault <dir> --scope <json|@file> --out <json>` now opens
  the Aster vault, summarizes the graph collection, writes
  `SummarizeResult` JSON, prints `SummarizeResult::Display`, and appends the
  `SUMMARIZE_INVOKED` provenance row to Aster `cf/ledger`, readable by
  `calyx ledger-tail --vault <dir> --last 1`.

FSV for #757 must read the JSON output, `cf/graph` rows, and the Aster ledger
tail bytes separately. A return value or passing test is not sufficient.
