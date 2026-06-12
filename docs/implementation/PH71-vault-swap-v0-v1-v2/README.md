# PH71 — V0 shadow → V1 flip → V2 calyx-only

**Stage:** S19 — Leapable Vault Swap  ·  **Crate:** `calyx-cli` / `calyx-mcp`  ·
**PRD roadmap:** P11  ·  **Axioms:** A18

> **This is the only required Leapable phase.** Discover-Vault hosting (V3) is
> optional and is NOT covered here. See `29_STAGE19_LEAPABLE.md` and PRD `15`.

## Objective

Replace the end-user SQLite/`sqlite-vec` Vaults with embedded Calyx — multi-lens,
kernel-grounded, guarded, provenanced — while Leapable's PostgreSQL control plane
stays **completely untouched**. Migration proceeds in three FSV-gated sub-phases
(V0 shadow → V1 flip → V2 calyx-only) so that each step is independently
byte-provable on a real Vault on aiwonder before the next step begins. At no point
does any Calyx code read, write, replace, shadow, or migrate PostgreSQL. The Vault
adapter implements the existing `vault-sqlite.ts` contract so the PostgreSQL control
plane sees zero behavioral change (PRD `15 §1`).

## Dependencies

- **Phases:** PH64 (migration tool — `calyx migrate vault`), PH33 (kernel index +
  `kernel_answer`), PH38 (τ calibration + guard), PH63 (calyx-mcp stdio surface)
- **Provides for:** Stage 19 exit (LEAPABLE predicate in BUILD_DONE, `03_PHASE_MAP
  §BUILD_DONE_mapping`)

## Current state (build off what exists)

PH64 delivers `calyx migrate vault <sqlite> <vault.calyx>` with chunk → 1-slot
constellation migration. PH33 delivers `kernel_answer`. PH38 delivers `Gτ` guard.
PH63 delivers MCP tool surface. The Vault adapter shim (`vault-sqlite.ts` contract
in Rust FFI / cbindgen boundary) is **greenfield** for this phase. `libcalyx`
embedding harness is **greenfield**.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-cli/src/leapable/shadow_harness.rs` | V0: embed `libcalyx` as shadow index; dual-write coordinator; recall comparator (≤500 lines) |
| `crates/calyx-cli/src/leapable/dual_write.rs` | V0: ingest path that writes both `sqlite-vec` and Calyx in one logical operation (≤500 lines) |
| `crates/calyx-cli/src/leapable/recall_comparator.rs` | V0: compares Ask results from `sqlite-vec` vs Calyx; parity metrics and gate (≤500 lines) |
| `crates/calyx-cli/src/leapable/read_flip.rs` | V1: flip Vault reads from `sqlite-vec` → Calyx; `sqlite-vec` demoted to shadow (≤500 lines) |
| `crates/calyx-cli/src/leapable/panel_guard_enable.rs` | V1: enable multi-lens panel + kernel/guard on the live Vault after flip (≤500 lines) |
| `crates/calyx-cli/src/leapable/round_trip_verifier.rs` | V1: byte-exact `.db` round-trip verification via readback (≤500 lines) |
| `crates/calyx-cli/src/leapable/shadow_removal.rs` | V2: remove `sqlite-vec` shadow; install default panels per Vault type (≤500 lines) |
| `crates/calyx-cli/src/leapable/production_fsv.rs` | V2: full-provenance readback + control-plane identical-response proof + PostgreSQL-untouched proof (≤500 lines) |
| `crates/calyx-cli/src/leapable/issue612_fsv.rs` | Issue #612 closure verifier: flipped-read p99 non-regression and widened `pg_dump` table diff (<=500 lines) |
| `crates/calyx-cli/src/leapable/mod.rs` | module root, re-exports (≤100 lines) |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `libcalyx` embedded shadow harness (V0) | PH64 |
| T02 | Dual-write ingest + recall-parity comparator (V0 gate) | T01 |
| T03 | Read-flip to Calyx + multi-lens panel/kernel/guard enable (V1) | T02 |
| T04 | Byte-exact `.db` round-trip migration verifier (V1 gate) | T03 |
| T05 | Remove `sqlite-vec` shadow + default-panels-per-vault (V2) | T04 |
| T06 | Production Vault calyx-only + control-plane-identical FSV + PostgreSQL-untouched proof (V2 gate) | T05 |
| T07 | Issue #612 verifier: persisted flipped-read latency samples plus widened PG snapshot (`creator_databases`, `queries`, `billing`, `marketplace`, `outbox`) | T03, T06 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Three sub-gates, each proven by byte readback on a real Vault on aiwonder:

1. **V0 shadow gate:** recall parity ≥ baseline on a real Vault — `calyx readback
   <vault.calyx>` shows ingested constellations byte-matching their SQLite source
   rows; comparator log shows Calyx recall ≥ `sqlite-vec` recall on the same query
   set.
2. **V1 flip gate:** A/B recall win (Calyx ≥ sqlite-vec), no latency regression;
   `calyx migrate vault <real.db> <vault.calyx>` round-trips byte-exact on content
   (every chunk's text hash and `chunk_id` matches source SQLite). Issue #612 adds
   a mechanical verifier for persisted live latency samples: flipped-read p99 must
   be no more than 105% of the sqlite-vec baseline p99.
3. **V2 calyx-only gate:** a real production Vault runs Calyx-only with full
   provenance; every Ask returns a LedgerRef-cited result; **control-plane
   queries/billing/listing for that Vault return identical results** before and
   after; `pg_dump` diff of the PostgreSQL instance shows **zero rows changed**.

PostgreSQL verified untouched: run the same `psql` queries against
`creator_databases`, `queries`, billing tables before and after every sub-gate —
responses must be byte-identical. Evidence attached to the PH71 GitHub issue.

Issue #612 widens the required PostgreSQL snapshot set to include
`marketplace` and `outbox` alongside `creator_databases`, `queries`, and billing.
The `pg_dump` bytes for all five table groups must match before and after.

## Risks / landmines

- **Contract names:** `database_name`, `chunk_id`, SQL table names in `vault-sqlite.ts`
  are code-contract identifiers — any rename breaks the control plane silently.
  All Calyx files must preserve these verbatim (PRD `15 §4`).
- **Vector mixing:** `LensId` content-addressing must be enforced at the FFI
  boundary; a model-version bump that reuses a `LensId` would silently corrupt
  recall. Fail closed with `CALYX_LENS_FROZEN_VIOLATION`.
- **Candidate text persistence:** the reranker candidate text must never be written
  to Aster or the Ledger — it is request-scoped only (PRD `15 §4`).
- **Embedded GPU-less path:** on end-user machines there is no CUDA; the CPU SIMD
  Forge + ONNX lenses must be the hot path; `libcalyx` must not fail or degrade if
  no GPU is present.
- **PostgreSQL state on aiwonder:** the existing leapable/postgres state must not be
  touched during any FSV run — use a test Vault copy, never the live DB.
- **EXDEV / ZFS rename atomicity:** Aster's atomic swap relies on `rename(2)`
  within the same ZFS dataset; confirm `CALYX_HOME` and the Vault dir share one
  dataset.
