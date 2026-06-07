# PH34 · T06 — FSV: ≥4 distinct scopes on a real corpus, each with measured recall

| Field | Value |
|---|---|
| **Phase** | PH34 — Multi-scope kernel |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/tests/fsv_multi_scope.rs` (≤500) |
| **Depends on** | T05 (all scope machinery complete), PH33-T05 (real corpora available on aiwonder) |
| **Axioms** | A21, A10 |
| **PRD** | `dbprdplans/08 §4b`, `08 §7` |

## Goal

Run `build_kernel` at **≥4 distinct scopes** on a real corpus on aiwonder, verify
that each scope produces its own measured `kernel_only_recall` and `grounded_fraction`
(which differ across scopes — no copy-paste of a global value), and write a
`ScopeKernelReport` for each. This is the byte-level FSV gate for PH34. Also verify
that `Union` kernel ≠ naive member union, and that bridge nodes are correctly
identified.

## Build (checklist of concrete, code-level steps)

- [ ] Create `tests/fsv_multi_scope.rs`; gated `#[cfg(feature = "fsv")]`.
- [ ] Load a real corpus from `$CALYX_HOME/datasets/` (verified checksum per PH69 MANIFEST).
- [ ] Run `build_kernel` on each of the 4+ scopes:
  1. `AllAssociations` — global kernel.
  2. `Collection(id)` — one collection from the corpus.
  3. `TimeWindow { t0, t1 }` — a time slice (past 30 days or similar).
  4. `Domain(anchor_kind)` — the sub-corpus anchored to a specific outcome type.
  5. (Optional 5th) `Union(Collection(id1), Collection(id2))` — bridge test.
- [ ] For each scope: run `kernel_recall_test` (PH33-T04) with `rng_seed=42`, `top_k=10`;
  record `kernel_only_recall`, `grounded_fraction`, `approx_factor`, `kernel_size`.
- [ ] Assert: each scope's `kernel_only_recall ≥ 0.90` (slightly relaxed from PH33's
  `0.95` for scope-specific kernels which may be smaller; the `AllAssociations` scope
  must still meet `0.95`).
- [ ] Assert: `grounded_fraction` values differ across scopes (not all 0.0 or all 1.0).
- [ ] Write one JSON per scope to `$CALYX_HOME/fsv/ph34_scope_<name>_<date>.json`.
- [ ] Print a summary table: scope name | kernel_size | recall | grounded_fraction | approx_factor.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] `#[test] #[cfg(feature = "fsv")] fn fsv_all_associations_scope()` — runs, recall ≥ 0.95, JSON written.
- [ ] `#[test] #[cfg(feature = "fsv")] fn fsv_collection_scope()` — runs, recall ≥ 0.90, JSON written.
- [ ] `#[test] #[cfg(feature = "fsv")] fn fsv_time_window_scope()` — runs (or skips with
  `CALYX_SCOPE_TEMPORAL_NOT_READY` if PH22/PH40 not yet done), JSON written or skip logged.
- [ ] `#[test] #[cfg(feature = "fsv")] fn fsv_domain_scope()` — runs, recall ≥ 0.90, JSON written.
- [ ] `#[test] #[cfg(feature = "fsv")] fn fsv_union_bridges()` — runs `Union` scope,
  `bridges()` returns non-empty list; `bridge_count` in report > 0.
- [ ] edge: checksum mismatch → `CALYX_DATASET_CHECKSUM_MISMATCH`; test skipped with println.
- [ ] fail-closed: any scope with `kernel_only_recall < 0.90` fails the test with the
  exact `ratio` and `scope_name` in the error message.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** JSON report files at `$CALYX_HOME/fsv/ph34_scope_*.json`; printed summary table.
- **Readback:**
  ```
  cargo test -p calyx-lodestar --features fsv fsv_multi_scope 2>&1 | tee /tmp/ph34_t06_fsv.txt
  cat $CALYX_HOME/fsv/ph34_scope_*.json
  ```
- **Prove:** ≥4 JSON files present (one per scope); each contains distinct `scope_name`,
  `kernel_only_recall`, and `grounded_fraction`; `AllAssociations` recall ≥ 0.95;
  at least 3 other scopes ≥ 0.90; summary table printed with 4+ rows;
  union-bridges test prints `bridge_count > 0`; all output attached to PH34 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH34 GitHub issue
- [ ] ≥4 `ScopeKernelReport` JSON files at `$CALYX_HOME/fsv/ph34_scope_*.json`
      with distinct recall + grounded_fraction values; attached to PH34 GitHub issue
- [ ] Summary table printed showing ≥4 scope rows
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
