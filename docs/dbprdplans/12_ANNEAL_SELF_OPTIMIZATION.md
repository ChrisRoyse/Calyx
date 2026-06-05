# 12 — Anneal: Self-Healing, Self-Learning, Self-Optimization

> **Living-system role:** homeostasis + healing + sleep — self-repair, learning from reality, and consolidation (A31 — DOCTRINE §1b)

Implements A14. The user's requirement: *"the entire database needs to self heal, self learn and self improve; the more it is used it needs to self optimize; all the math needs to self optimize and tune itself for whatever jobs a function is getting used on."* Anneal is the background nervous system doing this — safely, reversibly, logged.

## 0. Primary objective: maximize grounded intelligence (A32, `27`)

Anneal is **the optimizer of the Intelligence Objective `J`** (`27`). Its three loops below (heal/learn/optimize) all serve one end: **grow grounded intelligence as fast as safely possible.** "Recode itself to maximize intelligence" (the founder's mandate) is implemented here as **online parameter self-adjustment**: as new data arrives, Anneal re-fits the math's parameters (fusion weights, quant levels, index params, `τ`, materialization, online heads — `27 §5`) toward `J`, fastest-marginal-gain-first (`27 §3`), each change shadow-tested against held-out `J`, kept only on a measured gain with no tripwire regression, reversible and Ledger-logged. Performance (latency/compression/recall) is optimized *as a facet of* `J`, not as a competing goal (`27 §9`).

## 1. The three loops

| Loop | Cadence | Goal |
|---|---|---|
| **Self-heal** | continuous + on-fault | recover from corruption/drift/failure without data loss |
| **Self-learn** | online, per-mistake | improve predictions/calibration from observed outcomes |
| **Self-optimize** | adaptive, usage-driven | make math/indexes/storage faster and truer for *this* workload |

Every Anneal action obeys two invariants: **never regress a tripwire metric** (recall, FAR, latency SLO) and **always reversible + Ledger-logged** (A14/A15).

## 2. Self-heal

| Trigger | Action |
|---|---|
| Corrupt ANN/kernel/guard (derived) | rebuild from base+slots in background; serve `degraded` flag meanwhile (A16) |
| Corrupt base shard | fail reads closed on the range; restore from ZFS snapshot/restic; alert |
| Lens endpoint failing | mark `health=failing`; route to remaining lenses (graceful degradation); retry/backoff |
| Drifted `τ` (FAR creep) | Ward recalibrate; alert if drift exceeds bound |
| Lens signal decayed < 0.05 bits | auto-**park** the lens (keep, stop searching); alert (Assay-driven) |
| Stale derived beyond bound | prioritized rebuild |

Graceful degradation is the rule (inherited from ContextGraph): a failing component degrades search quality and raises a flag; it never returns wrong-but-confident results.

## 3. Self-learn (online mistake-closure)

The JEPA "wrong only once" loop, as a database service (absorbed from ContextGraph `mistake_log`/`replay_buffer`/`online_head_state`/heal):

```
on observed outcome that contradicts a trusted prediction:
  1. append MistakeLog{cx, predicted, observed, anchor, ts}
  2. add cx to ReplayBuffer (prioritized by surprise = |predicted−observed|)
  3. in a "sleep"/heal pass: update small ONLINE HEADS (predictor, calibrator, fusion weights)
     via EWC++-style continual update — never touching frozen lenses (A4)
  4. re-assert: the same mistake must not recur on replay (regression check)
  5. Ledger-log the heal; expose "what I learned" deltas
```

What adapts: online predictor heads, `τ` calibration, RRF/weight profiles, intent classifier, cross-term materialization plan, kernel membership. What never adapts: frozen lens weights (A4) and persisted constellations (A15) — only *derived* structures learn.

Reality reward: an arriving `Anchor` (test pass, tie formed, thumbs) is a reward signal Anneal uses to retune — *learning from reality, not from an LLM* (the video's claim, made concrete). More usage → more anchors → faster convergence.

## 4. Self-optimize (the math tunes itself)

The user's "all the math self-optimizes for whatever job a function is used on." Anneal autotunes across four layers:

| Layer | What's tuned | How |
|---|---|---|
| **Kernels (Forge)** | matmul tile sizes, distance kernels, batch sizes, dtype (fp16/bf16/fp8) per slot dim & GPU occupancy | online microbenchmark + cache best config per `(op, shape, dtype, device)`; CubeCL/cuBLAS autotune (`13`) |
| **Index** | HNSW `ef`/`M`, DiskANN beamwidth, SPANN posting cutoffs, quantization level per slot | bandit search against measured recall/latency on live queries; promote/demote per workload |
| **Materialization (Loom)** | which cross-terms are eager vs lazy; which Concat keys get an index | re-run the plan as query patterns & bits shift (`06`) |
| **Storage** | compaction cadence, tiering (hot↔cold), codebook refresh, prefetch | usage counters; move cold slots to `archive`, hot to NVMe/VRAM frontier |

Mechanism: a **per-shape config cache** keyed `(operation, input_shape, dtype, device, recall_target)` → best-known parameters, refreshed by a low-rate exploration policy (ε-greedy/Thompson) that always A/B's a candidate against the incumbent on **live traffic** and only promotes on a measured win with no tripwire regression. Every promotion is Ledger-logged and revertible.

**Result:** a function called a million times on 768-d code vectors converges to a different kernel/index config than one called on 1024-d civic vectors — each path optimizes itself for its actual job, no human tuning. This kills the user's "I need full code every single time" pain: the *tuning* is also automatic.

## 5. Lens proposal (closing the `I(panel;outcome)` deficit)

When Assay reports `I(panel; anchor)` ≪ `H(anchor)` (the panel can't predict an outcome), Anneal:
1. localizes the deficit (which outcome class, which inputs) via Assay's per-sensor attribution,
2. **proposes a new lens** (commission-on-corpus or algorithmic synthesis, `05 §6`) targeting the missing bits,
3. profiles the candidate (Registry capability card), admits it only if it clears the differentiation contract (≥0.05 bits, ≤0.6 corr),
4. hot-adds it (no re-embed) and re-measures sufficiency.

The paper's "the fix is a panel of the right sensors, not more training" — automated as a database maintenance loop. (Absorbed from ContextGraph `embedder_proposal`/`instrument_proposal`/`embedder_falsification`.)

## 6. Safety rails (binding)

- **Tripwires:** recall@k, guard FAR/FRR, search p99, ingest p95. Any Anneal change that crosses a tripwire auto-reverts.
- **Shadow-first:** index/quant/kernel changes run in shadow and must beat the incumbent on a held-out query replay before promotion.
- **Reversible:** every change keeps the prior artifact until the new one is proven; rollback is one pointer swap.
- **Logged:** Ledger `kind=Anneal` entry for every promotion/park/recalibration/proposal — self-optimization is fully auditable (A15).
- **Bounded compute:** Anneal runs in a capped background budget (VRAM/CPU) so it never starves serving; on aiwonder it yields to the resident TEI/marketplace services.

## 7. Anneal API (summary; full in `18`)

```
status(vault) -> {heals, learns, optimizations, proposals, tripwire_state}
record_outcome(cx_id, anchor) -> feeds mistake-closure + reward
propose_lens(vault, anchor) -> CandidateLens + capability card
autotune_report(vault) -> per-(op,shape) best configs + recent A/Bs
rollback(change_id) -> reverts a promotion
set_tripwire(metric, bound)
```

**One sentence:** Anneal is why Calyx gets faster and truer the more it's used — healing corruption, learning from real outcomes without touching frozen lenses, proposing the lens that closes a sufficiency gap, and autotuning every kernel and index to the actual job, all reversible and logged.

Source heritage: ContextGraph `mejepa` heal/mistake/replay/online-head + `embedder_proposal`; EWC++ continual learning; bandit autotuning.
