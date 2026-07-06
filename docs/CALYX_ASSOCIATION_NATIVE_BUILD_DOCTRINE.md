# Calyx Association-Native Build Doctrine

This doctrine applies to Calyx project work in any domain. It is the operating
translation of the builder handbook into engineering rules for issues, docs,
code, and FSV.

## Foundational Rule

Build from first principles:

1. Decompose the project into atomic, independently measurable units.
2. Store those atoms and their provenance in Calyx.
3. Compute every base association that the current scope makes possible.
4. Differentiate associations against grounded outcomes in bits.
5. Distill the minimal kernel.
6. Compose search, guard, prediction, imputation, mining, and reporting from
   that kernel.

Records are not the intelligence. The association structure among measured
atoms is the intelligence substrate. A feature, field, molecule, protein, DNA
sequence, paper, trial, drug, disease, endpoint, safety signal, and outcome is
useful only after it is represented as a Calyx atom with provenance and linked
by typed associations that can be read back.

The binding method is:

```text
atoms -> all base associations -> differentiate -> kernel -> compose
```

Do not skip any step. If a run cannot cover all atoms or all base associations
in a scope, the run must narrow and name the scope instead of implying complete
coverage. "All associations" means all base relationships the declared scope
and current Calyx capabilities make measurable, with deficits routed to new
data-acquisition or capability issues.

## Modeling Rules

- Use Calyx as the source of truth. Do not create a side store for data that
  Calyx must reason over; add the missing Calyx capability instead.
- Embed latent content: free text, images, audio, video, code, scientific
  sequences, descriptions, and generated summaries.
- Encode explicit values: numbers, categories, booleans, ordinals, timestamps,
  identifiers, relationships, and measurements.
- For hybrid records, do both. Keep exact structured encoders and semantic
  embedding slots separate in the same constellation.
- Never flatten a panel. Slots stay typed and independent through search,
  guard, bits, kernel, and report surfaces.
- Every derived association, admission, roster, gate, plan, truth-reference, and
  provenance claim must be stored as Calyx/Aster rows or edges and preserve
  enough source ids, hashes, and row counts to trace it back to the physical
  Calyx rows or admitted source bytes. JSON, reports, logs, manifests, FBIN,
  I8BIN, and similar file artifacts are diagnostic or import/export bytes only;
  they never decide association, admission, roster membership, or phase exit.
- Calyx/Aster database rows are the only association source of truth. If an
  association, gate, plan, or roster decision cannot be read back from the
  database itself, it is not admitted. Anything persisted outside the database
  must be treated as a cache, source import, export, or diagnostic transcript.

Structured values are first-class intelligence inputs. Numbers, categories,
booleans, ordinals, timestamps, identifiers, references, relations, graph
position, recurrence, and measured outcomes must be represented with frozen
deterministic encoders when they are explicit. Text labels, descriptions,
notes, abstracts, and generated row summaries should be embedded as additional
slots when their meaning is latent. Hybrid records should carry both families
in the same constellation, still no-flatten.

Every missing data class is an engineering task, not a reason to infer around
the gap. If a biomedical claim needs trials, safety, mechanisms, perturbation
signatures, targets, variants, disease ontology, outcome labels, or negative
controls, acquire those source bytes, ingest them into Calyx with provenance,
and compute their associations with the rest of the substrate.

## Association Rules

- Count all base associations within the bounded scope, not only convenient
  or hand-picked pairs.
- Make scope explicit. If a run is bounded, partial, sampled, or incomplete,
  the artifact must say so and must not be described as full-corpus proof.
- Associations are ranked evidence and hypotheses until differentiated against
  grounded outcomes and validated by the required gates.
- Do not claim derived `C(N,2)` signal past the data-processing ceiling of the
  measured panel/outcome information.
- Refusals, gaps, and deficits are findings. Persist them and route them to
  the next issue or lens/data acquisition task.
- Use typed association surfaces. A drug-disease co-mention, drug-target edge,
  target-disease validation, transcriptomic reversal, adverse-event signal,
  clinical-trial intervention-condition row, and literature citation are
  different evidence instruments and must not be collapsed into one generic
  "association" without preserving type and provenance.
- Build composition from the kernel. Search, answer, guard, hypothesis mining,
  imputation, consequence prediction, atlas publishing, and lowered artifacts
  must name the kernel or graph generation they were derived from.

## Grounding Rules

- Grounding means a real anchored outcome or power-proven instrument, not a
  plausible association.
- Bits, sufficiency, known-positive/negative, time-split, safety, and
  counter-evidence gates are required before promoting a biomedical association
  beyond hypothesis status.
- A clinical, treatment, cure, or actionability claim requires all applicable
  validation, safety, and counter-evidence gates to pass with physical
  readback. Association-only inference can prioritize leads; it cannot by
  itself become a clinical recommendation.
- Missing data does not justify a fallback. Acquire the data, add the Calyx
  storage/ingest/readback capability, or fail closed with a structured error.

Biomedical claim ladder:

1. Association-only output is a ranked research lead.
2. Evidence-backed output is a provenance-backed hypothesis.
3. Falsification-retained output is a stronger hypothesis that survived the
   available counter-evidence gates.
4. Mechanism/safety/outcome-validated output may become a preclinical or
   clinical-review candidate.
5. Clinical actionability or cure language requires the relevant clinical,
   safety, regulatory, and human-review gates. Calyx must refuse that claim
   until the source-of-truth artifacts prove those gates.

## Execution Rules

- GitHub issues are live state. Claim, update, split, and close issues with
  artifact-backed evidence.
- Every task that changes association state must produce or update durable
  Calyx/Aster source-of-truth rows: association edges, ledger entries, graph
  lifecycle rows, admission rows, truth-reference rows, or roster/plan rows.
  Docs and issues record context and decisions; report files are diagnostics or
  export surfaces, not authority.
- Full State Verification reads the persisted Calyx/Aster source of truth after
  the write. Command success, logs, JSON files, and tests are supporting
  evidence, not proof.
- Gate-bearing partitioned RRF recall is a DB-only control path: plan,
  timeline, A37 admission, and accepted-reference truth must all be
  Calyx/Aster rows read back from their CF roots. JSON cards, manifests, logs,
  and vector files may support diagnostics, import/export, or raw vector bytes,
  but they cannot satisfy `--recall-floor`.
- If a prerequisite is missing, create the atomic issue and wire it into the
  dependency order before doing downstream mining.
- Prefer root-cause Calyx capability work over one-off scripts. Scripts are
  acceptable as bounded FSV/prototype tools only when their outputs are
  imported into Calyx or tracked as source artifacts.
- Keep the issue tree atomic. If completing one issue reveals missing source
  data, a missing Calyx storage/readback primitive, a missing gate, or a
  missing validator, file or update the specific issue before downstream
  claims depend on it.
- For substantial discovery runs, persist the authoritative result, findings,
  admission state, and readback proof as Calyx/Aster rows. Operator summaries may
  point at those rows, but they are not the substrate for future mining and must
  not replace database readback.

## Runtime and Embedder Policy

- Use the simplest measured path that keeps work on the GPU. On aiwonder-class
  RTX 5090 / CUDA 13.3 hardware, the dense text default is a resident Blackwell
  TEI service using `float16`/FP16, fail-closed GPU execution, bounded batching,
  and database readback of model id, dtype, device, batch limits, VRAM, latency,
  and per-lens bits.
- For GeForce RTX 50X0, prefer the Blackwell 12.0 TEI/Candle lane (`sm_120`).
  A wrapper or local image name is not proof; the proof is Calyx/Aster readback
  plus startup evidence for `Float16`, CUDA device placement, host `libcuda`,
  and healthy latency. The upstream TEI `120-*` image line is the target when
  rolled out, but do not churn a healthy resident service without a measured
  admission win.
- BF16, INT8, FP8, FP4, ONNX, Candle, and alternate TEI images are allowed only
  when a Calyx/Aster admission row proves GPU execution and improves measured
  speed, VRAM density, or bits-per-VRAM for the bounded corpus. No CPU fallback
  is admissible for a GPU runtime.
- Do not keep a large dense embedder because it is familiar. If a smaller frozen
  lens provides equivalent semantic utility with better measured density, replace
  or park the larger lane. Conversely, do not swap the active dense lane when the
  measured blocker is panel diversity, roster collapse, or missing association
  families rather than dense semantic latency or VRAM.

## Biomedical Discovery Implication

For the #867 biomedical program, the required shape is:

1. Evidence atoms: diseases, drugs, targets, genes, variants, pathways,
   phenotypes, trials, transcriptomic signatures, outcomes, safety signals,
   assays, literature claims, molecules, proteins, and DNA.
2. Calyx storage: every atom, source row, outcome, and validation signal belongs
   in Calyx with provenance and graph lifecycle state.
3. Association substrate: typed edges and collection-local CSR/readback over
   accepted graph generations.
4. Differentiation: known-positive/negative, time-split, bits, sufficiency,
   reversal, target/outcome, safety, and counter-evidence gates.
5. Composition: all-pair typed miners, domain hunts, synergy miners, human
   review atlas, and Oracle/kernel prediction surfaces.

The goal is maximum grounded healing intelligence: find, rank, falsify, and
explain human-healing hypotheses. The system must not skip the gates needed to
turn a hypothesis into a stronger claim.

## Default Constants To Preserve Unless Recalibrated

- Association yield per input: `N + C(N,2) + 1`.
- Bit floor: about `0.05` bits.
- Correlation ceiling: about `0.6`.
- Minimum paired samples for MI/calibration: about `50`, unless a small-sample
  posterior or explicit provisional status is used.
- Sufficiency: `I(panel; anchor) >= H(anchor)`.
- Kernel recall target: about `0.95`.
- Reciprocal-rank fusion default: `K = 60`.

Changing these constants is allowed only when the new value is calibrated,
documented, and verified against persisted artifacts.
