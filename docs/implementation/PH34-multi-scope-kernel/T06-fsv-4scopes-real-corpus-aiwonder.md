# PH34 T06 - FSV: >=4 distinct scopes on a real corpus, each measured

| Field | Value |
|---|---|
| **Phase** | PH34 - Multi-scope kernel |
| **Stage** | S6 - Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/tests/fsv_multi_scope.rs`; `tests/support/multi_scope_fsv.rs`; `tests/support/multi_scope_fsv/union_check.rs` (all <=500 lines) |
| **Depends on** | T05 (all scope machinery complete), PH33-T05 (real corpora available on aiwonder) |
| **Axioms** | A21, A10 |
| **PRD** | `dbprdplans/08 §4b`, `08 §7` |

## Goal

Run `build_kernel` at >=4 distinct scopes on a real corpus on aiwonder, verify
that each scope produces its own measured `kernel_only_recall` and
`grounded_fraction`, and write one `ScopeKernelReport` JSON per scope. This is
the byte-level FSV gate for PH34. It also verifies that a `Union` kernel is not
the naive member union and that bridge nodes are identified.

## Status

Done on aiwonder on 2026-06-08 for #238. The manual FSV trigger used the real
SciFact corpus from `$CALYX_HOME/datasets/` and wrote source-of-truth JSON files
under `$CALYX_HOME/fsv/`.

Readback hashes below are current after #331 added explicit `recall_tuning`
fields to the PH34 real-corpus reports:

| SoT file | SHA-256 |
|---|---|
| `/home/croyse/calyx/fsv/ph34_scope_all_20260608.json` | `41d85bebe87602ec478f86ac77b4930593e1a91b9502e4465f2e531d456978eb` |
| `/home/croyse/calyx/fsv/ph34_scope_collection_a_20260608.json` | `d43523c4168c45ba2931048a850d7c5c4cdbcb6a4f0675ac812e5fa9886cbe3a` |
| `/home/croyse/calyx/fsv/ph34_scope_time_window_20260608.json` | `d50e208178a3d0b9c9c53557e8c640283323428b472cb27b3c47223bddf8d104` |
| `/home/croyse/calyx/fsv/ph34_scope_domain_20260608.json` | `a6b503c1da1301641f811ac1b0378e00222781d42df50e33dcbc9dd878b70043` |
| `/home/croyse/calyx/fsv/ph34_scope_union_20260608.json` | `66ff7f5d29a2e32457e7cfc04b28b5b1c99687e848180d385d397eed46763aa1` |
| `/home/croyse/calyx/fsv/ph34_scope_summary_20260608.json` | `9ede67ec5866ee0c8345d3b26600276c4e42cb9bc74faf0105a61008f4d779af` |
| `/home/croyse/calyx/fsv/ph34_t06_fsv_20260608.log` | `57587c7c36ed74ef31b2636587aac724db610cc8875a7d6353e84c48c7217ee2` |

Measured scope rows:

| Scope | Rows | Kernel size | Recall | Grounded fraction |
|---|---:|---:|---:|---:|
| `AllAssociations` | 180 | 151 | `0.95000005` | `0.03311258` |
| `Collection(collection_a)` | 125 | 87 | `0.9` | `0.03448276` |
| `TimeWindow(1700000030..1700000119)` | 90 | 69 | `0.90000004` | `0.0` |
| `Domain(label:ph34-real-scope)` | 60 | 48 | `0.95` | `0.104166664` |
| `Union(collection_a, collection_b)` | 180 | 151 | `0.95000005` | `0.03311258` |

These measured kernel sizes are the PH34 source of truth for the scoped runs.
They are not normalized to, or evidence for, a universal ≈1% raw kernel; each
scope reports its own final kernel size, recall, grounded fraction, and tuning
fields.

Union diagnostic readback: `mfvs_not_naive_union=true`, naive member union size
`2`, union kernel size `1`, bridge list non-empty.

## Build

- [x] Create `tests/fsv_multi_scope.rs`; gated `#[cfg(feature = "fsv")]`.
- [x] Load a real corpus from `$CALYX_HOME/datasets/`; checksums are verified by
      the shared PH33 real-corpus loader.
- [x] Run `build_kernel` on these 5 scopes:
  1. `AllAssociations`.
  2. `Collection(collection_a)`.
  3. `TimeWindow { t0: 1700000030, t1: 1700000119 }`.
  4. `Domain(label:ph34-real-scope)`.
  5. `Union(Collection(collection_a), Collection(collection_b))`.
- [x] For each scope: run `kernel_recall_test` with `rng_seed=42`, `top_k=10`;
      record `kernel_only_recall`, `grounded_fraction`, `approx_factor`, and
      `kernel_size`.
- [x] Assert `AllAssociations` recall >=0.95 and all other scopes recall >=0.90.
- [x] Assert `grounded_fraction` values differ across scopes.
- [x] Write one JSON per scope to `$CALYX_HOME/fsv/ph34_scope_<name>_20260608.json`.
- [x] Print a summary table with scope, kernel size, recall, grounded fraction,
      approx factor, and bridge count.

## FSV

- **Trigger:** `CALYX_HOME=/home/croyse/calyx cargo test -p calyx-lodestar --features fsv fsv_multi_scope_real_corpus_aiwonder -- --ignored --nocapture --test-threads=1`
- **SoT:** JSON report files at `$CALYX_HOME/fsv/ph34_scope_*_20260608.json`
  and the trigger log at `$CALYX_HOME/fsv/ph34_t06_fsv_20260608.log`.
- **Readback:** `ls -l`, `sha256sum`, and `cat` were run on every report file
  from aiwonder after the trigger.
- **Proven:** 5 JSON files exist; each contains a distinct `scope_name`,
  measured `kernel_only_recall`, and measured `grounded_fraction`; all recall
  gates pass; grounded fractions vary; bridge nodes are non-empty; union MFVS is
  not the naive member union.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder.
- [x] File line-count gate passes; all new `.rs` files are <=500 lines.
- [x] FSV evidence attached to #238.
- [x] >=4 `ScopeKernelReport` JSON files exist at `$CALYX_HOME/fsv`.
- [x] Summary table printed showing >=4 scope rows.
- [x] No PH34 anti-pattern: union scope runs MFVS on the union graph, not
      `members_a union members_b`; all claims are backed by source-of-truth
      file readback.
