# 09 — Ward: the `Gτ` Constellation Guard

> **Living-system role:** immune system / self-vs-non-self — the boundary of the grounded self against foreign or injected input (A31 — DOCTRINE §1b)

Implements A12. Ward is Teleological Constellation Training reused at query/write time: the panel becomes a **frozen alignment target** and every model-produced vector is gated by a **per-output cosine guard `Gτ`**. Stops drift and prompt injection; turns "novelty" into a new safe region instead of a failure.

## 1. The mechanism (from the video + ClipCannon + Polis)

For a produced vector (an AI output, a candidate generation, an incoming query) Ward measures **cosine similarity to the matched constellation's slot** and compares to a calibrated threshold `τ`:

```
guard(produced_slot_vec, matched_cx, slot_k):
  s = cos(produced_slot_vec, matched_cx.slot_k)
  if s ≥ τ[slot_k]   -> PASS  (inside the trusted region for this axis)
  else               -> FAIL  (outside scope) -> becomes a NEW constellation / new safe region
```

- A constellation passes only if **all required slots** pass their per-slot `τ` (required-slot set is per domain; e.g. Polis required the identity slots).
- A `FAIL` is not an error — it means *"this is outside what I've grounded, so it opens a new region"* (the video's "new dot → whole new constellation"). Ward records it as a novel region (provenance-tagged) rather than forcing it into a wrong neighbor. Simultaneously **anomaly detection, injection defense, and continual learning.**

## 2. Why this stops prompt injection / drift

An injected or hallucinated output lands **outside** every trusted constellation's `τ` ball on the load-bearing slots, so it `FAIL`s the guard and cannot be silently accepted as matching a trusted region. The ClipCannon "stomp prompt injection" result: you don't classify the attack, you geometrically require outputs to sit inside grounded regions. (Polis `0009-constellation-guard`, `0806-i10-no-flatten-hard-gate`, ContextGraph `mejepa-tct`.)

The **no-flatten** rule (A3) is essential: per-slot guarding is far stronger than guarding one flattened vector, because an attack that fools the average can't fool every axis at once.

## 3. Threshold calibration (`τ`)

`τ` is **not** a magic constant; Ward calibrates it per domain against grounded outcomes (mirrors Polis `tau_calibration`, ContextGraph `conformal`/`threshold_calibration_provenance`):


| Step | Method |
|---|---|
| Collect | for an anchored set, gather cos(produced, matched) for known-good (should pass) and known-bad (injection/OOD/wrong) outputs |
| Calibrate | choose `τ[slot_k]` per slot to hit a target operating point — **conformal**: guarantee a bounded false-accept rate at a chosen confidence; or ROC point per the domain's cost of false-accept vs false-reject |
| Per-slot | each slot gets its own `τ` (identity slots strict, stylistic slots loose) |
| Provenance | store `τ`, the calibration corpus hash, the estimator, the achieved FAR/FRR, and ts (`11`) — `τ` is auditable, not asserted |
| Refresh | Anneal recalibrates as the corpus drifts; a `τ` whose FAR creeps up triggers re-calibration + alert (`12`) |

Default starting `τ` ≈ **0.7** cosine (the ClipCannon/`mejepa-tct` operating value), but the **calibrated** value governs; the constant is only a cold-start prior.

## 4. GuardProfile object

```
GuardProfile {
  guard_id, panel_version, domain,
  tau: Map<SlotId, f32>,             // per-slot calibrated threshold
  required_slots: Vec<SlotId>,       // which must pass for the constellation to pass
  policy: AllRequired | KofN{k},     // how slot passes combine
  calibration: { corpus_hash, estimator, far, frr, confidence, ts },  // provenance
  novelty_action: NewRegion | Quarantine | RejectClosed,  // what FAIL does
}
```

## 5. Generation-time integration (the loop)

Calyx is the gate between a model and the trusted store:

```
model produces candidate -> Forge.measure(candidate, required_lenses) -> per-slot vectors
  Ward.guard(...) per required slot:
     PASS all required -> accept; write/answer; provenance "guarded:pass"
     FAIL -> per novelty_action:
        NewRegion  -> store as novel constellation, flag novel_region, queue for grounding (anchor)
        Quarantine -> hold for human/agent review, do not serve as trusted
        RejectClosed -> refuse (fail closed) for high-stakes domains
```

Exactly the video's "is your AI's output inside what I allow it to generate?" check, now a database primitive. It also feeds Anneal's mistake-closure: a `NewRegion` that later gets a *bad* anchor becomes a tightening signal for `τ`.

## 5b. Identity-locked generation (the paper's measured proof)

Ward's headline application is **identity-locked generation**: pin a generator (voice, writing style, persona) to a grounded constellation and require every output to stay inside the `Gτ` ball on the identity slots. The paper's measured anchors are exactly Ward verdicts:

- **Voice:** a reproduced voice at **0.961 mean WavLM speaker-similarity** (encoder-matched; **DNSMOS 3.93/3.93**). In Calyx terms: a `speaker` lens (WavLM) slot whose `Gτ` requires cos ≥ calibrated τ to the target speaker's constellation → identity-locked TTS. The 0.961 is the achieved in-region similarity; speaker verification is the anchor.
- **Style:** a published style model that **holds character under prompt injection**, with an **emergent zero-shot transfer to Golden-Age Spanish**. In Calyx terms: required style-slots guard the persona; an injection that would break character lands outside τ on the style slots → `FAIL`/quarantine, so character holds. The zero-shot transfer shows the frozen style lens measures an axis (voice/register) that generalizes beyond its training language — a designable-lens result.

Concrete `AnchorKind`s: `SpeakerMatch` and `StyleHold` (speaker verification, persona consistency). Identity-locked generation = Ward + a speaker/style lens + calibrated τ; nothing bespoke per project.

## 6. Relationship to the other engines

| Engine | Interaction |
|---|---|
| Assay | bits decide which slots are *required* (load-bearing slots get strict `τ`; ≥0.05-bit slots only) |
| Lodestar | the kernel constellations are the strongest guard anchors (grounded); guard against kernel-near regions first |
| Loom | per-slot guarding depends on no-flatten (A3) and uses the same normalized cosine as Agreement |
| Sextant | a query can itself be guarded (reject OOD queries) and search can be restricted to in-`τ` regions |

## 7. Honesty & limits

- `Gτ` guards **geometric** scope, not truth: it ensures an output is *inside a grounded region*, which is necessary-not-sufficient for correctness. Ward reports "in-region", and grounding (anchors) is what makes the region trustworthy.
- An adversary who can match every required slot's `τ` is, by construction, producing something inside the grounded distribution on every measured axis — Ward raises the bar to "fool all N lenses simultaneously," not to "perfect."
- Calibration MUST be against grounded outcomes; an uncalibrated `τ` is tagged `provisional` and high-stakes domains MUST refuse to run on a provisional guard (fail closed).

## 8. Ward API (summary; full in `18`)

```
calibrate(vault, domain, anchored_set, target_far) -> GuardProfile
guard(vault, produced_slots, matched_cx?) -> {pass, per_slot: [(slot, cos, tau, pass)], action}
guard_query(vault, query_slots) -> Pass | OOD{nearest, gap}
novel_regions(vault, since?) -> [novel constellations awaiting grounding]
guard_health(guard_id) -> {far, frr, drift, last_calibrated}
```

**One sentence:** Ward is the boundary — requiring every AI output to sit inside a grounded region on every load-bearing axis, turning prompt-injection defense, drift detection, and continual learning into one calibrated cosine gate.
