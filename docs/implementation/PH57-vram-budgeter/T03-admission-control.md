# PH57 · T03 — Admission control — split/queue/fail, `CALYX_FORGE_VRAM_BUDGET`

| Field | Value |
|---|---|
| **Phase** | PH57 — VRAM budgeter + admission control |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/vram/admission.rs` (≤500) |
| **Depends on** | T01 (budgeter), T02 (LRU eviction) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §2` |

## Goal

Implement the admission controller that intercepts every large VRAM dispatch and applies the
three-tier policy: (1) if splitting the batch would fit within the budget, split and run in
sub-batches; (2) if the batch is medium and a queue slot is available, queue it (bounded
queue, deadline-driven); (3) if neither fits nor can be queued within deadline, fail closed
with `CALYX_FORGE_VRAM_BUDGET`. Never silently OOM. This is the coordination layer between
the budgeter/eviction (T01/T02) and the dispatch path.

## Build (checklist of concrete, code-level steps)

- [ ] Define `enum AdmitDecision { Split { sub_batch_size: usize }, Queue { deadline: Instant }, Fail }` in `admission.rs`
- [ ] Define `struct AdmissionController { budgeter: Arc<VramBudgeter>, registry: Arc<Mutex<GpuBlockRegistry>>, queue_cap: usize, split_min_batch: usize }` 
- [ ] Implement `AdmissionController::decide(&self, requested_bytes: usize, batch_size: usize, deadline: Instant) -> AdmitDecision`:
  - If `budgeter.can_allocate(requested_bytes)` succeeds → `AdmitDecision::Split { sub_batch_size: batch_size }` (no-op split, proceed full)
  - Else try eviction: `registry.evict_until(requested_bytes)`; if eviction creates enough space → proceed
  - Else if `requested_bytes / 2 >= split_min_batch` → `AdmitDecision::Split { sub_batch_size: batch_size / 2 }` (halve batch)
  - Else if queue has capacity and deadline is in the future → `AdmitDecision::Queue { deadline }`
  - Else → `AdmitDecision::Fail` (returns `CALYX_FORGE_VRAM_BUDGET`)
- [ ] Implement `AdmissionController::run_with_admission<F, R>(&self, bytes: usize, batch: usize, deadline: Instant, f: F) -> Result<R, CalyxError>` — drives the decide-then-act loop; splits recursively until `sub_batch_size >= split_min_batch`; collects partial results; assembles final result
- [ ] Bounded queue: `VecDeque<QueuedDispatch>` with `cap == queue_cap`; if full → `Fail` immediately
- [ ] Emit `CALYX_FORGE_VRAM_BUDGET` with payload `{ requested_bytes, available_bytes, budget_bytes }` for diagnostics
- [ ] Add admission decision counters to `VramStats`: `splits_total`, `queued_total`, `failed_total`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: 1 GiB budget, request 512 MiB → `decide` returns `Split { sub_batch_size: full_batch }` (fits, no split needed)
- [ ] unit: 1 GiB budget with 900 MiB already reserved, request 512 MiB → eviction of 412 MiB LRU block → proceeds; or if no evictable block → `Split { sub_batch_size: batch/2 }`
- [ ] unit: budget 1 GiB, nothing evictable, request 2 GiB → `split_min_batch` reached after K halvings, then queue not full → `Queue`; if queue also full → `Fail` → `CALYX_FORGE_VRAM_BUDGET`
- [ ] unit: `run_with_admission` with a batch of 8, splits to 4 then 2; assembles 4 partial results into final; all 8 items processed
- [ ] proptest: `forall budget, requests: Vec<(bytes, batch)>` — decision is always one of the three tiers; `failed_total + queued_total + splits_total == len(requests)`; no silent panic
- [ ] edge: `deadline` already past → `AdmitDecision::Fail` immediately (no queue attempt)
- [ ] edge: `requested_bytes == 0` → `Split { sub_batch_size: batch }` (zero-byte request always admitted)
- [ ] fail-closed: queue at capacity, budget at cap, nothing evictable, deadline in future → `CALYX_FORGE_VRAM_BUDGET` (not enqueue → overflow)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `VramStats::failed_total` counter and Prometheus `calyx_forge_vram_budget_exceeded_total`
- **Readback:** `calyx readback --metric forge_vram_budget_exceeded_total` during concurrent TEI soak (T06)
- **Prove:** dispatch 50 concurrent over-budget requests → `failed_total >= 1`; `forge_vram_budget_exceeded_total` counter increments; no OOM in `dmesg`; split requests produce correct results (verify byte-parity of split vs non-split output on a deterministic input).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden set
- [ ] FSV evidence (readback output / screenshot) attached to the PH57 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
