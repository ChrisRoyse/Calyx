# PH43 — Tripwires + Shadow-First + Reversible/Rollback

**Stage:** S10 — Anneal + Intelligence Objective J  ·  **Crate:** `calyx-anneal`  ·
**PRD roadmap:** `12 §6`, `27 §4`  ·  **Axioms:** A14, A15

## Objective

Implement the safety substrate every Anneal action runs under: metric tripwires
that auto-revert any change crossing a guarded bound; a shadow-first execution
model that requires a candidate to beat the incumbent on held-out replay before
promotion; and a one-pointer-swap rollback mechanism so every change is instantly
reversible. All promotions/reverts are Ledger-logged with `kind=Anneal`. This
phase makes every subsequent Anneal action safe by construction.

## Dependencies

- **Phases:** PH24 (search fusion + provenance — provides recall@k + p99 metrics),
  PH16 (autotune config cache — provides the per-`(op,shape,dtype,device)` config
  slot that rollback swaps)
- **Provides for:** PH44, PH45, PH46, PH47, PH48 (all Anneal actions run under
  this substrate)

## Current state (build off what exists)

`calyx-anneal` crate is a 9-line stub; greenfield. `Cargo.toml` references it
but the crate contains only `lib.rs` with a module placeholder. No tripwire,
shadow, or rollback logic exists. All Anneal actions from PH44 onward depend on
this phase being complete.

**Anneal invariants (binding for every card in S10):**
- Every Anneal action is reversible + tripwire-guarded + Ledger-logged.
- Bounded background compute budget — yields to serving traffic and the resident
  TEI services on aiwonder (:8088/:8089/:8090).

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/tripwire.rs` | Metric tripwire registry: recall@k, guard FAR/FRR, search p99, ingest p95; crossing threshold auto-reverts; hysteresis |
| `src/shadow.rs` | Shadow-first execution: run candidate in shadow against held-out replay; promote only if beats incumbent on all tripwire metrics |
| `src/rollback.rs` | Artifact store (prior pointer kept); rollback = one atomic pointer swap; rollback log |
| `src/budget.rs` | Background compute budget enforcer: CPU/VRAM ceiling, yield to serving + TEI |
| `src/ledger_anneal.rs` | Ledger `kind=Anneal` writer: every promotion/revert/proposal writes a chained entry |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Tripwire registry (metrics + thresholds + hysteresis) | — |
| T02 | Shadow executor (held-out replay + beat-incumbent check) | T01 |
| T03 | Rollback store (prior artifact + pointer swap) | T01 |
| T04 | Background budget enforcer (CPU/VRAM yield) | — |
| T05 | Ledger `kind=Anneal` writer | T03 |
| T06 | Integration: bad-change auto-revert FSV scenario | T01, T02, T03, T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Inject a deliberately-bad change (e.g., lower HNSW recall by corrupting `ef`
config) → tripwire fires → Ledger entry with `kind=Anneal` and `action=revert`
is written → prior artifact pointer is restored → `calyx readback ledger` prints
the revert entry with original pointer hash → `xxd` the config slot confirms the
prior value is byte-exact. Both the tripwire-fired Ledger row AND the restored
pointer must be present; no serving-path metric may regress.

## Risks / landmines

- **Hysteresis band must be calibrated** — too tight → oscillation; too loose →
  bad changes persist. Use `±5%` of the threshold as the default band; make it
  configurable via `set_tripwire`.
- **Shadow replay must use a seeded, deterministic held-out set** — never live
  traffic order (non-deterministic); seed all RNG in tests.
- **Pointer swap must be atomic** on the config cache slot (use `Arc<ArcSwap>`
  or equivalent); partial swap = data race.
- **Budget enforcer** must not add latency to serving-path hot loops — check
  budget on the background task scheduler, not inline.
- **Ledger stub** (PH35 not yet merged at this point) — write to the Ledger CF
  directly; the real hash-chain comes in PH35 but the `kind=Anneal` format must
  be forward-compatible.
