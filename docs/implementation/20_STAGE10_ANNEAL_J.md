# Stage 10 — Anneal + Intelligence Objective J (PH43–PH48)

The background nervous system: self-heal, self-learn (mistake-closure), self-
optimize (autotune), lens proposal — all serving one objective, **maximize
grounded intelligence `J`**, fastest-first, every change reversible + tripwire-
guarded + Ledger-logged. Lands in `calyx-anneal`. **Living-system role:**
homeostasis + healing + sleep + the drive.

---

## PH43 — Tripwires + shadow-first + reversible/rollback
- **Objective.** The safety substrate every Anneal action runs under.
- **Deps.** PH24, PH16.
- **Deliverables.** `tripwire.rs` (recall@k, guard FAR/FRR, search p99, ingest
  p95), `shadow.rs` (run candidate in shadow, beat incumbent on held-out replay
  before promote), `rollback.rs` (keep prior artifact; rollback = one pointer
  swap), Ledger `kind=Anneal` on every change.
- **Key tasks.** any change crossing a tripwire auto-reverts; hysteresis;
  bounded background compute budget (yields to serving + TEI).
- **FSV gate.** a deliberately-bad change is auto-reverted (tripwire fires,
  read the Ledger entry + the restored pointer); rollback restores byte-state.
- **Axioms/PRD.** A14, A15, `12 §6`, `27 §4`.

## PH44 — Self-heal (rebuild derived, degrade flags)
- **Objective.** Recover from corruption/drift without data loss.
- **Deps.** PH43, PH33.
- **Deliverables.** triggers (corrupt ANN/kernel/guard → background rebuild +
  `degraded` flag; failing lens → route to remaining; drifted τ → recalibrate;
  decayed lens → park).
- **Key tasks.** rebuild derived from base+slots; graceful degradation (never
  wrong-but-confident); restore base from snapshot/restic on corruption.
- **FSV gate.** flip an ANN/kernel index byte → read degrades + rebuilds in
  background, **no data loss** (read base intact); a killed lens endpoint →
  search degrades, doesn't hang.
- **Axioms/PRD.** A16, `12 §2`, `24 §7` (rows 12,16).

## PH45 — Mistake-closure + online heads + replay buffer
- **Objective.** The JEPA "wrong only once" loop as a DB service; learn from real
  anchors, never touch frozen lenses.
- **Deps.** PH44.
- **Deliverables.** `MistakeLog`, `ReplayBuffer` (surprise-prioritized),
  `OnlineHeadState` (predictor/calibrator/fusion weights, EWC++-style update),
  regression re-assert.
- **Key tasks.** observed contradiction → log → replay → update online heads →
  same mistake must not recur; only derived structures learn (A4/A15 intact).
- **FSV gate.** feed a contradicting outcome → online head updates → the same
  mistake does **not** recur on replay (read before/after predictions); frozen
  lens weights unchanged (hash stable).
- **Axioms/PRD.** A4, A14, `12 §3`, `03 §6`.

## PH46 — Autotune loops (index/quant/fusion/materialization)
- **Objective.** The math tunes itself per workload across four layers.
- **Deps.** PH45, PH16.
- **Deliverables.** bandit autotuner over Forge kernels, index params (`ef`/`M`/
  beamwidth/cutoffs), quant level per slot, Loom materialization plan, fusion
  weights — A/B on live traffic, promote on measured win only.
- **Key tasks.** per-`(op,shape,dtype,device)` config; ε-greedy/Thompson with
  hysteresis; every promotion reversible + logged.
- **FSV gate.** **1e6-query soak** on aiwonder → **p99 ↓ ≥20%, no recall
  regression, no oscillation** (read the metric series + Ledger A/Bs).
- **Axioms/PRD.** A14, `12 §4`, `19 §4`.

## PH47 — Lens proposal (sufficiency deficit)
- **Objective.** When `I(panel;anchor)≪H(anchor)`, propose the lens that closes
  the deficit — "the fix is the right sensors, not more training."
- **Deps.** PH46, PH30.
- **Deliverables.** `propose_lens(anchor)` (localize deficit via attribution →
  commission-on-corpus or algorithmic synthesis → profile → admit only if
  contract clears → hot-add → re-measure sufficiency).
- **Key tasks.** deficit localization; candidate profiling (capability card);
  differentiation gate (≥0.05 bits, ≤0.6 corr).
- **FSV gate.** on a known-insufficient panel, a proposed lens that clears the
  contract **raises measured sufficiency** (read `I(panel;anchor)` before/after);
  a non-qualifying candidate is rejected.
- **Axioms/PRD.** `12 §5`, A7, `07 §4`.

## PH48 — J objective + growth curve + intelligence_report
- **Objective.** The measurable composite `J` Calyx maximizes, the intelligence-
  gradient priority queue, and the growth curve — grounded + DPI-capped +
  Goodhart-defended.
- **Deps.** PH47.
- **Deliverables.** `J.rs` (the composite from `27 §2`), `gradient.rs`
  (`ΔJ/cost` priority queue → next_best_action), `intelligence_report`,
  `growth_curve`, weight tuning, Goodhart checks (held-out + Gτ + cross-lens
  anomaly), penalties (redundant/ungrounded/goodhart).
- **Key tasks.** every `+` term is a real grounded measurement; DPI ceiling caps
  info terms; ungrounded excluded; compression/retrieval are facets not
  competing goals.
- **FSV gate.** `J` measured; **growth_curve rises** on a real corpus under the
  loop; a gamed change fails held-out validation (read the curve + the rejected
  promotion); no data deleted to "optimize".
- **Axioms/PRD.** A32, A2, A8, `27` (all).

---

## Stage 10 exit
Calyx gets faster and truer the more it's used — healing, learning from real
outcomes without touching frozen lenses, proposing the lens that closes a
sufficiency gap, autotuning every kernel/index, and climbing a measured,
grounded, Goodhart-defended intelligence objective — PRD `SELFOPT` +
`INTELLIGENCE`.
