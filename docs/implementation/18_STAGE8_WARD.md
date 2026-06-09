# Stage 8 — Ward Gτ Guard (PH37–PH39)

**Status:** pending. Tracked by Stage 8 epic #257 and exit issue #280; PH37-PH39
atomic tasks are #258-#279.

Teleological Constellation Training at query/write time: the panel is a frozen
alignment target and every model-produced vector is gated by a per-output cosine
guard `Gτ`. Stops drift + prompt injection; turns novelty into a new safe region.
Lands in `calyx-ward`. **Living-system role:** immune system / self-vs-non-self.

---

## PH37 — Gτ guard math + GuardProfile
- **Objective.** Per-slot cosine gate with all-required (or KofN) pass logic;
  no-flatten enforced.
- **Deps.** PH22 (slots/lenses), PH13 (cosine).
- **Deliverables.** `guard.rs` (`cos(produced_k, matched_k) ≥ τ_k`),
  `GuardProfile { tau: Map<SlotId,f32>, required_slots, policy, calibration,
  novelty_action }`, per-slot verdict breakdown.
- **Key tasks.** require **every** required slot to pass (no flattened vector,
  A3); `CALYX_GUARD_OOD` on fail; verdict carries per-slot `(cos,tau,pass)`.
- **FSV gate.** an output passing the average but failing one required slot is
  **rejected** (read per-slot verdict); no-flatten path is the only path.
- **Axioms/PRD.** A12, A3, `09 §1/§2/§4`.

## PH38 — τ calibration (conformal) + novelty→new-region
- **Objective.** Calibrate `τ` per slot against grounded outcomes with a bounded
  false-accept rate; a FAIL opens a new region, not a silent accept.
- **Deps.** PH37, PH28 (grounded outcomes).
- **Deliverables.** `calibrate.rs` (conformal: bound FAR at confidence 1−α; per-
  slot; provenance: corpus_hash, estimator, FAR/FRR, ts), `novelty.rs`
  (NewRegion|Quarantine|RejectClosed), drift monitor hook (Anneal).
- **Key tasks.** ROC/conformal per slot; identity slots strict, stylistic loose;
  uncalibrated τ → `provisional`, high-stakes refuses; `CALYX_GUARD_PROVISIONAL`.
- **FSV gate.** **injection corpus blocked ≥99% at the calibrated FAR** (real
  prompt-injection set on aiwonder); a valid-novelty input → new region (read
  the novel constellation + the calibration provenance).
- **Axioms/PRD.** A12, A2, `09 §3`, `19 §4`.

## PH39 — Identity-locked generation (speaker/style)
- **Objective.** Pin a generator (voice/style/persona) to a grounded
  constellation; every output must stay inside the `Gτ` ball on identity slots.
- **Deps.** PH38, PH19 (speaker/style lenses).
- **Deliverables.** `SpeakerMatch`/`StyleHold` anchor handling; identity-slot
  required-set; integration with `guard_generate`.
- **Key tasks.** commission a WavLM speaker lens + a style lens (HF); require
  cos ≥ calibrated τ on identity slots; injection that breaks character →
  quarantine.
- **FSV gate.** a target-speaker constellation guards TTS output (in-region
  similarity measured, e.g. against VoxCeleb); an injection that would break
  persona lands outside τ on the style slots → quarantined (read verdicts).
- **Axioms/PRD.** `09 §5b`, A12, `05 §7`.

---

## Stage 8 exit
Ward is the boundary — every AI output must sit inside a grounded region on every
load-bearing axis, making injection defense, drift detection, and continual
learning one calibrated cosine gate, plus injection-proof identity-locked
generation — PRD `GUARD`. Also powers TCT dedup (Stage 9) and Anneal's mistake-
closure.
