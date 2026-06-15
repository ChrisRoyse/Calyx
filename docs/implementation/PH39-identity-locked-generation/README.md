# PH39 — Identity-Locked Generation (Speaker / Style)

**Stage:** S8 — Ward Gτ Guard  ·  **Crate:** `calyx-ward`  ·
**PRD roadmap:** P6  ·  **Axioms:** A12

## Objective

Pin a generator (voice, writing style, persona) to a grounded constellation and
require every output to stay inside the `Gτ` ball on the identity slots.
Concretely: commission a WavLM speaker lens and a style lens (via HuggingFace
candle or ONNX runtime), add `SpeakerMatch` and `StyleHold` anchor kinds to the
schema, define an identity-slot required-set, and implement `guard_generate()`
as the generation-time integration loop. An injection that would break character
lands outside τ on the style slots and is routed to `Quarantine`. Target
achieved speaker-similarity: 0.961 mean WavLM cosine in-region (the paper's
measured value, `09 §5b`).

## Dependencies

- **Phases:** PH38 (calibrated `GuardProfile` + `NoveltyHandler`), PH19
  (candle-local + ONNX runtimes — WavLM and style model loaded here)
- **Provides for:** PH41 (TCT dedup uses the identity-slot required-set),
  PH72 (streaming ingest uses identity-locked generation for persona-consistent
  synthesis)

## Current state (build off what exists)

`calyx-ward` is active, not a stub: PH37 T01/T02 (#258/#259) shipped the
profile, verdict, and error surfaces, and PH37 T03 (#260) adds the first
`guard()` math slice before PH38 calibration and PH39 identity work build on
it. PH19 (candle/ONNX runtimes) is required for the WavLM and style lenses;
stub with mock lens outputs for unit tests; integrate real models on aiwonder
for the FSV run. PH39 T01 (#269) is signed off: `calyx-core` already exposed
`SpeakerMatch` and `StyleHold`, and `calyx-ward::IdentityProfile::new()`
builds a constructor-validated/deserializer-validated identity profile with
cached normalized matched-slot vectors. Durable evidence:
`/home/croyse/calyx/data/fsv-issue269-identity-profile-20260609`. PH39 T02
(#270) is signed off with durable WavLM/ORT readbacks under
`/home/croyse/calyx/data/fsv-issue270-speaker-lens-20260609-ef729f8-ort126-sm120`.
PH39 T03 (#271) is signed off with the pinned
`AnnaWegmann/Style-Embedding` ONNX style lens under
`/home/croyse/calyx/models/style/` and durable readbacks under
`/home/croyse/calyx/data/fsv-issue271-style-lens-20260609-a43e546-ort126-sm120`.
PH39 T04 (#272) is signed off with `guard_generate()` accepted/novel/rejected
paths and Ledger Guard provenance readback under
`/home/croyse/calyx/data/fsv-issue272-guard-generate-20260609-3bce50c`.
PH39 T05 (#273) is signed off with a real `deepset/prompt-injections` corpus row
quarantined on numeric style slot `9` under
`/home/croyse/calyx/data/fsv-issue273-ph39-t05-20260609-8d2572b-ort126-sm120`.
PH39 T06 (#274) is signed off with deterministic eSpeak v2 target-speaker
fixtures under `/home/croyse/calyx/data/identity_fsv/speaker_tts_espeak_ng_20260609_v2`
and durable readbacks under
`/home/croyse/calyx/data/fsv-issue274-ph39-t06-20260609-8e29b51-v2-cpu-ort126`:
mean WavLM speaker similarity is `0.9882728457450867` against the
`0.9610000252723694` target, in-region min is `0.9850643873214722`, five
cross-speaker samples are `Rejected`, and the Stage 8 summary JSON reports all
PH37/PH38/PH39 checks passing.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/identity.rs` | `SpeakerMatch` + `StyleHold` anchor handling; identity-slot required-set; `IdentityProfile` wrapper; `IdentityProfile::new()` |
| `src/generate.rs` | `guard_generate()` loop: produce → embed → guard → route; provenance "guarded:pass" tag |
| `src/speaker_lens.rs` | WavLM speaker lens adapter using `ort` with pinned aiwonder model bytes; `embed_speaker(audio, sample_rate) -> Vec<f32>` |
| `src/style_lens.rs` | Style lens adapter (HF candle or ONNX); `embed_style(text) -> Vec<f32>` |
| `tests/identity_fsv.rs` | Deterministic FSV tests: speaker similarity target, style-hold injection quarantine |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends | Status |
|---|---|---|---|
| T01 | `SpeakerMatch` + `StyleHold` anchor kinds + `IdentityProfile` | — | DONE / FSV #269 |
| T02 | WavLM speaker lens adapter (`embed_speaker`) | T01 · PH19 | DONE / FSV #270 |
| T03 | Style lens adapter (`embed_style`) | T01 · PH19 | DONE / FSV #271 |
| T04 | `guard_generate()` integration loop + provenance tag | T03 | DONE / FSV #272 |
| T05 | Identity-slot injection → quarantine FSV | T04 | DONE / FSV #273 |
| T06 | Speaker similarity target FSV (0.961 mean WavLM cos) | T05 | DONE / FSV #274 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Two proofs on aiwonder:

1. **Speaker identity locked:** a target-speaker constellation guards TTS
   output; `guard_generate()` returns verdicts showing in-region WavLM cos ≥
   calibrated τ on the `speaker` slot; mean over the test set ≥ 0.961. Write
   per-slot verdict JSON to a durable aiwonder evidence root and read the bytes
   back with `xxd` or `calyx readback`; stdout is only a captured artifact.

2. **Style injection quarantined:** an injection prompt designed to break
   persona lands outside τ on the style slots; `NoveltyHandler` routes to
   `Quarantine`; `NoveltyRecord.status == Quarantined` is readable from the
   durable sink's `novel_records()` output. Read via `calyx readback` or `xxd`.

Both durable readbacks and their hashes are attached to the PH39 GitHub issue.

## Risks / landmines

- WavLM checkpoint download (HF Hub) must be pinned to a content hash on
  aiwonder (`/home/croyse/calyx/models/wavlm/`); never re-download at test time.
- The 0.961 mean speaker-similarity target is measured on the in-region test set
  (matched speaker), not on cross-speaker pairs — test selection matters.
- Style lens must hold character under injection — test with at least one real
  injection prompt from the corpus, not only synthetic vectors.
- `guard_generate()` must not re-embed the matched constellation on each call;
  cache the matched-slot vectors in `IdentityProfile` at construction time.
- Missing identity fixtures, style profiles, TTS samples, or model data are setup
  work, not a successful skip. T05/T06 must prove durable aiwonder readback.
