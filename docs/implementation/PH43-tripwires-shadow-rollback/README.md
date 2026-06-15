# PH43 - Tripwires + Shadow-First + Reversible/Rollback

Stage: S10 - Anneal + Intelligence Objective J
Crate: `calyx-anneal`
PRD roadmap: `12 section 6`, `27 section 4`
Axioms: A14, A15

## Objective

Implement the safety substrate every Anneal action runs under: metric tripwires
that auto-revert any change crossing a guarded bound; a shadow-first execution
model that requires a candidate to beat the incumbent on held-out replay before
promotion; and a one-pointer-swap rollback mechanism so every change is instantly
reversible. All promotions/reverts are Ledger-logged with `kind=Anneal` once the
PH43 Ledger card lands.

## Dependencies

- PH24 search fusion + provenance provides recall@k and p99 metrics.
- PH16 autotune config cache provides the per-`(op, shape, dtype, device)` config
  slot that rollback swaps.
- T01 tripwire registry gates shadow promotion.

## Current State

`calyx-anneal` has PH42 recurrence scheduling plus PH43 T01, T02, T03, T04,
and T05:

- T01 persists tripwire thresholds under `<vault>/.anneal/tripwire.toml`, exposes
  `check`/`set_tripwire`/`status`, and provides
  `calyx readback config tripwire --vault <dir>` for byte-backed inspection.
- T02 provides deterministic held-out replay sampling, `ShadowExecutor`,
  budget-tick enforcement, tripwire-gated promote/revert verdicts, and durable
  FSV verdict artifacts.
- T03 provides durable rollback snapshots and live artifact pointers in Aster CF
  `anneal_rollback`, with WAL-backed pointer swaps for prepare, promote,
  rollback, and commit.
- T04 provides the non-blocking `BudgetEnforcer`, vault budget config readback,
  CPU/VRAM RAII handles, conservative NVML-unavailable fallback, and budget
  status artifacts.
- T05 provides `AnnealLedger`, a hash-chained `EntryKind::Anneal` writer over
  the existing PH35 ledger path, with Aster `ledger` CF storage, recent reads,
  change-id lookup, and fail-closed oversized-payload handling.

The integrated bad-change auto-revert scenario remains the open PH43 card.

## Anneal Invariants

- Every Anneal action is reversible, tripwire-guarded, and Ledger-logged.
- Background compute is bounded and yields to serving traffic and the resident
  TEI services on aiwonder (:8088/:8089/:8090).
- A candidate never mutates the live path during shadow execution.

## Deliverables

| File | Responsibility | Status |
|---|---|---|
| `src/tripwire.rs` | Metric tripwire registry: recall@k, guard FAR/FRR, search p99, ingest p95, hysteresis | Done |
| `src/shadow.rs` | Shadow-first execution: run candidate against held-out replay; promote only if it beats incumbent on all tripwire metrics | Done |
| `src/rollback.rs` | Artifact store; rollback as one atomic pointer swap | Done |
| `src/budget.rs` | Background compute budget enforcer: CPU/VRAM ceiling, yield to serving + TEI | Done |
| `src/ledger_anneal.rs` | Ledger `kind=Anneal` writer for every promotion/revert/proposal | Done |

## Tasks

| Card | Title | Depends | Status |
|---|---|---|---|
| T01 | Tripwire registry (metrics + thresholds + hysteresis) | - | Done |
| T02 | Shadow executor (held-out replay + beat-incumbent check) | T01 | Done |
| T03 | Rollback store (prior artifact + pointer swap) | T01 | Done |
| T04 | Background budget enforcer (CPU/VRAM yield) | - | Done |
| T05 | Ledger `kind=Anneal` writer | T03 | Done |
| T06 | Integration: bad-change auto-revert FSV scenario | T01, T02, T03, T05 | Open |

## FSV Exit Gate

The phase is done only when this is byte-proven on aiwonder:

1. Inject a deliberately bad change, such as lowering HNSW recall by corrupting
   `ef` config.
2. Confirm the tripwire fires.
3. Confirm a Ledger entry with `kind=Anneal` and `action=revert` is written.
4. Confirm the prior artifact pointer is restored.
5. Run `calyx audit --vault <dir> --kind anneal`,
   `calyx scan --cf ledger --vault <dir>`, and
   `calyx readback --cf ledger --vault <dir> --seq <n>` to read the revert
   entry with the original pointer hash.
6. `xxd` the config slot and confirm the prior value is byte-exact.

Both the tripwire-fired Ledger row and restored pointer must be present; no
serving-path metric may regress.

## Risks / Landmines

- Hysteresis must be calibrated: too tight causes oscillation, too loose lets bad
  changes persist. Default to 5% of the threshold, configurable via
  `set_tripwire`.
- Shadow replay must use a seeded deterministic held-out set, never live traffic
  order.
- Pointer swap must be atomic on the config cache slot; partial swap is a data
  race. T03 uses a `RwLock` state guard plus one Aster WAL batch for each
  snapshot/live-pointer mutation.
- Budget enforcement must not add latency to serving-path hot loops; check
  budget on the background task scheduler, not inline.
- T05 writes through the PH35 `LedgerAppender`; do not reintroduce side ledger
  row formats or bypass the hash-chain appender.
