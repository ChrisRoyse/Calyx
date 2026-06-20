# PH68 — DiskANN dense + SPANN sparse

**Stage:** S17 — Scale: DiskANN + SPANN  ·  **Crate:** `calyx-sextant`  ·
**PRD roadmap:** P10  ·  **Axioms:** A14, A15, A16, A26, A32

## Objective

Extend `calyx-sextant` from in-RAM HNSW (1e6–1e7 constellations) to disk-resident
billion-scale indexes: DiskANN on-disk graph for dense slots and SPANN
(centroids-in-RAM, posting-lists-on-NVMe) for sparse slots. Add dual-DiskANN for
asymmetric slots and the kernel-first 3-hop funnel (kernel-of-regions → region →
cx) that keeps huge-vault queries sublinear. Wire Anneal to autotune beamwidth and
posting-cutoff online. Deliver a 1e8–1e9-cx server vault that answers within the
search SLO: **KernelFirst@1e8 p99 < 25 ms** (`10 §8`). Recall evidence in this
phase is ANN/index correctness only: overlap with exact or accepted-reference
nearest-neighbor truth. It is not grounded-intelligence evidence and cannot close
a "system works" claim without separate Assay `I(panel;oracle)` and Lodestar
grounding-kernel coverage over a valid real outcome.

> **SCALE HONESTY (binding, `17 §3.4`):** Billion-scale is a **SERVER** target,
> running on aiwonder's `hotpool` NVMe + HDD (RTX 5090, 1.5 TB hot). It is
> **NEVER** a laptop or embedded promise. Embedded vaults top out at 1e6–1e7 with
> in-RAM HNSW. Do not add any API or documentation that implies billion-scale on
> a consumer device.

## Dependencies

- **Phases:** PH23 (per-slot HNSW — provides the in-RAM ANN baseline, index trait,
  and slot-level index lifecycle that DiskANN replaces at server scale),
  PH25 (inverted index — provides the in-RAM sparse posting lists that SPANN
  replaces at server scale),
  PH46 (Anneal autotune loops — provides the Anneal hook DiskANN/SPANN
  beamwidth/cutoff autotune registers into)
- **Provides for:** PH70 (intelligence validation on real billion-scale corpora),
  PH72 (streaming ingest into large server vaults)

## Current state (build off what exists)

`calyx-sextant` has per-slot HNSW (PH23), inverted index (PH25), DiskANN graph
format/search, token DiskANN + segmented MaxSim, and concat cross-term DiskANN.
The remaining PH68 work is SPANN, dual DiskANN, kernel-first routing, and the
full 1e8+ ANN/index SLO soak. The vault physical layout (`04 §3`) reserves
`idx/slot_NN.ann/` for dense DiskANN, `idx/slot_NN.token.ann/` for multi-vector
token DiskANN, `idx/xterm.concat.ann/` for materialized concat xterm DiskANN,
and `idx/slot_NN.sparse/` for SPANN.

## PH68 GDELT Scale Roster Template (#801, #796, #787)

For GDELT/civic text-scale streams, the current gate-eligible roster template is
the #801 scale-audit panel proven on aiwonder at:

`/home/croyse/calyx/fsv/issue801-gdelt-entity-roster-20260620T0610Z/gdelt_roster_happy.json`

Readback: `accepted=true`, `content_lens_count=11`, `gpu_content_lens_count=5`,
`temporal_sidecar_count=1`, `temporal_counts_toward_content_floor=false`,
`temporal_lane_role=time_manipulation_walk_forward_backward_as_of_sidecar`, and
`rejected_count=0`.

| Role | Lens | Family | Runtime/provider | Placement | Effective batch |
|---|---|---|---|---|---|
| Time control, not content | `temporal-as-of-time-manipulation-sidecar` | `temporal_sidecar` | `algorithmic:scalar;cpu_explicit` | CPU | 1 |
| Content | `semantic-gte-base-tei` | `dense_semantic` | TEI `:8088` resident GPU service | GPU | 8 |
| Content | `domain-modernbert-legal-tei` | `dense_semantic` | TEI `:8090` resident GPU service | GPU | 8 |
| Content | `semantic-potion-base-8m` | `static_lookup_semantic` | `static_lookup_mmap;cpu_explicit` | CPU | 1 |
| Content | `semantic-all-minilm-l6-v2-candle` | `dense_semantic` | Candle CUDA, no CPU fallback | GPU | 1 |
| Content | `semantic-multilingual-e5-base-fastembed` | `dense_semantic` | ONNX CUDA EP, no CPU fallback | GPU | 8 |
| Content | `semantic-bge-small-en-v1-5` | `dense_semantic` | ONNX CUDA EP, no CPU fallback | GPU | 8 |
| Content | `a37-byte-char-features` | `byte_char` | `algorithmic:byte-features;cpu_explicit` | CPU | 1 |
| Content | `a37-lexical-sparse-keywords` | `lexical_sparse` | `algorithmic:sparse-keywords;cpu_explicit` | CPU | 1 |
| Content | `a37-late-interaction-token-hash` | `late_interaction_token` | `algorithmic:token-hash;cpu_explicit` | CPU | 1 |
| Content | `gdelt-cameo-event-code` | `entity_cameo_graph` | `algorithmic:gdelt-cameo;cpu_explicit` | CPU | 1 |
| Content | `gdelt-actor-geo-entity` | `entity_cameo_graph` | `algorithmic:gdelt-actor-geo;cpu_explicit` | CPU | 1 |

This roster satisfies the #787 >=10-lens floor and the #796 template mandate for
the GDELT scale path, while keeping temporal/time capture as a forward/backward
/ as-of traversal sidecar instead of a content-lens substitute. It also satisfies
A37 D1 family span for this domain: dense general/domain, static semantic,
byte/character, lexical sparse, late-interaction token, and entity/CAMEO graph.
The non-neural content-feature rows must persist
`signal_kind=deterministic_content_feature`; stale `signal_kind=algorithmic`
artifacts are rejected by the A35/A37 gate surfaces so placeholders cannot
masquerade as content lenses.
`calyx assay stream-fbin` and `calyx assay i8bin-ensemble-card` now default to
`--mode gate`, so full encodes and A37 readouts must carry eligible gate
evidence before they can stand as gate evidence. Homogeneous baseline/control
runs must use `--diagnostic` or `--baseline`; they still write streamed bytes and
the redundancy/PID matrix but remain `diagnostic_only`.

`calyx assay corpus-build` and `calyx assay stream-fbin` isolate each measured
lens in a short-lived worker process. This is required for ONNX CUDA runtimes:
the registry intentionally avoids unsafe ORT CUDA provider teardown in-process,
so the parent must not accumulate leaked provider state across a 10-lens panel.
The parent fails closed if a worker exits nonzero, omits its report, or reports
row counts that do not match the selected corpus. The worker report files are
part of the FSV source of truth and record per-slot PID, row counts, elapsed
time, manifest, and the final vector paths.

Current aiwonder FSV roots for this contract:

- Corpus-build 10-semantic happy path:
  `/home/croyse/calyx/fsv/issue803-corpusbuild-isolated-currenthead-20260620T124058Z`.
- Stream-FBIN 10-semantic happy path:
  `/home/croyse/calyx/fsv/issue803-stream-worker-10k-currenthead-20260620T130036Z`.
- Worker edge audit:
  `/home/croyse/calyx/fsv/issue803-worker-edge-fsv-20260620T130529Z`.

Custom ONNX text lenses must not derive padding length from the longest row in
the current batch. That made vectors depend on batch composition and forced the
legacy SciBERT control lens to `max_batch=1`. The runtime now groups inputs by
stable per-row sequence buckets and reassembles output order, but #812 proved
that BERT-family dynamic int8 graphs can still drift below the strict
`min_batch_cosine=0.999` default probe gate. The accepted replacement
domain/scientific lens is `domain-scincl-onnx-fp32`. Future runs should use the
stable aiwonder manifest:

`/home/croyse/calyx/lenses/commissioned/domain-scincl-onnx-fp32/lensforge.manifest.json`

The first-class FSV proof root for that same lens is:

`/home/croyse/calyx/fsv/issue812-scincl-fp32-firstclass-20260620T114251Z/commission/domain-scincl-onnx-fp32/lensforge.manifest.json`

aiwonder readback for the first-class `onnx-fp32` commission shows
`runtime=onnx`, `dtype=f32`, `license=mit`, `max_batch=64`, `placement=gpu`,
and `gpu_process_observed=true`. The strict scale-audit report at
`/home/croyse/calyx/fsv/issue812-scincl-fp32-firstclass-20260620T114251Z/reports/scale-audit-b64-min8.json`
is accepted with `effective_batch_size=64`, `min_cosine=0.9999995`, and
`rejected_count=0`. The replacement also passed the multi-embedder roster floor
at
`/home/croyse/calyx/fsv/issue812-scincl-11content-scale-byte-20260620T121315Z/scale-audit-11content-b64.json`:
`accepted=true`, `content_lens_count=11`, `gpu_content_lens_count=7`,
`rejected_count=0`, and temporal capture remains a
`time_manipulation_walk_forward_backward_as_of_sidecar` that does not count
toward the content floor. Future PH68/#803 stream-FBIN runs should replace
`domain-scibert-scivocab-uncased` with this SciNCL FP32 manifest unless they are
explicitly running the old SciBERT baseline for comparison.

Boundary evidence for the same roster:

- Content floor edge:
  `/home/croyse/calyx/fsv/issue801-gdelt-edge-content-floor-20260620T0625Z/content_floor_edge.json`
  read back `accepted=false`, `content=11`, `temporal=1`, and
  `CALYX_LENS_SCALE_CONTENT_FLOOR` when the floor was raised to 12.
- GPU floor edge:
  `/home/croyse/calyx/fsv/issue801-gdelt-edge-gpu-floor-20260620T0630Z/gpu_floor_edge.json`
  read back `accepted=false`, `gpu=5`, and
  `CALYX_LENS_SCALE_GPU_CONTENT_FLOOR` when the floor was raised to 6.
- Invalid GDELT actor/geo edge:
  `/home/croyse/calyx/fsv/issue801-gdelt-edge-invalid-dim-20260620T0635Z/invalid_dim_edge.json`
  read back the invalid manifest (`dim=0`) and the aggregate rejection with the
  worker stderr tail containing `CALYX_LENS_CONFIG_INVALID`.

Final gate log for the implementation bytes:

`/home/croyse/calyx/fsv/issue801-gdelt-final-gate-20260620T0640Z/gate.log`

Gate readback: CUDA 13.3 / RTX 5090 detected; `cargo fmt --check`,
`git diff --check`, line-count gate, `cargo check -p calyx-cli`,
`cargo test -p calyx-registry algorithmic -- --nocapture`,
`cargo test -p calyx-cli lens_commands::scale_audit -- --nocapture`,
`cargo build -p calyx-cli --features cuda`, and
`cargo clippy -p calyx-cli --all-targets --features cuda -- -D warnings` all
completed.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-sextant/src/index/diskann/graph.rs` | On-disk graph format: node layout (vector + neighbor list co-located for I/O locality), page-aligned blocks, mmap reader, graph builder (Vamana-style greedy insert + prune) |
| `crates/calyx-sextant/src/index/diskann/search.rs` | Beam search over on-disk graph: beamwidth-tuned BFS, I/O prefetch, raw-f32 rescore from cold sidecar; `DiskAnnSearch` impl of `SlotIndex` |
| `crates/calyx-sextant/src/index/diskann/token.rs` | Token DiskANN over flattened multi-vector tokens; candidate token hits are grouped by document and reranked with segmented MaxSim from raw token bytes |
| `crates/calyx-sextant/src/index/diskann/token_sidecar.rs` | Token DiskANN sidecars: `docs.cdt`, `token_docs.u32`, and `tokens.f32` with byte-readable document segments and token-to-document ordinals |
| `crates/calyx-sextant/src/index/diskann/concat.rs` | DiskANN over materialized `xterm` Concat rows, with `keys.cdx` preserving `(CxId, slot_a, slot_b, Concat)` identity for hits |
| `crates/calyx-sextant/src/index/diskann/dual.rs` | Dual-DiskANN for asymmetric slots: `asym_a` and `asym_b` graph pair, directional dispatch, dual-beam search, merge of asymmetric hit lists |
| `crates/calyx-sextant/src/index/spann/centroids.rs` | SPANN centroid index: k-means clustering into centroids (held in RAM), centroid ANN (tiny HNSW), centroid-to-posting-list map persisted to disk |
| `crates/calyx-sextant/src/index/spann/posting.rs` | SPANN posting lists on NVMe: varint+zstd block encoding, page-aligned I/O, append writer, random-access reader; eviction when RAM budget exceeded |
| `crates/calyx-sextant/src/index/funnel.rs` | Kernel-first 3-hop funnel for huge vaults (1e8+): kernel-of-regions → region ANN → cx ANN; `KernelFirstSearch` dispatch over the three tiers; `KernelFirst@1e8` p99 < 25 ms SLO |
| `crates/calyx-sextant/src/index/autotune.rs` | Anneal autotune hook: `BwPostcutoffTuner` observes p99 latency + recall@10; adjusts beamwidth and posting-cutoff via Anneal bandit; tripwire if recall drops below floor |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | DiskANN on-disk graph format + builder | — |
| T02 | DiskANN beam search + raw-f32 rescore | T01 |
| T03 | SPANN centroids-in-RAM + posting-lists-on-NVMe | — |
| T04 | Dual-DiskANN for asymmetric slots | T01, T02 |
| T05 | Kernel-first 3-hop funnel for huge vaults (1e8+) | T02, T03 |
| T06 | Anneal autotune of beamwidth/posting-cutoff + 1e8-cx SLO soak FSV | T02, T03, T05 |
| Gap #604 | Token DiskANN + MaxSim and concat xterm DiskANN | T01, T02 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

On aiwonder (`hotpool` NVMe, `/zfs/hot/calyx/`):

1. Build a synthetic 1e8-cx server vault with DiskANN graphs in
   `idx/slot_NN.ann/` and SPANN lists in `idx/slot_NN.sparse/`. Verify physical
   presence: `ls -lh /zfs/hot/calyx/<vault>/idx/slot_00.ann/` and
   `ls -lh /zfs/hot/calyx/<vault>/idx/slot_00.sparse/` — both directories must
   contain byte-populated files (non-zero size).
2. Run `calyx bench search --vault <vault> --strategy KernelFirst --n 1000
   --report p99` on aiwonder. Observed p99 must be **< 25 ms** (the
   `KernelFirst@1e8 p99 < 25 ms` SLO from `10 §8`). Recall artifacts must read
   back `metric_class=ann_correctness` and `grounded_phase_exit_eligible=false`.
3. Verify beamwidth and posting-cutoff were autotuned: `calyx anneal status
   --vault <vault> --tuner bw_postcutoff` prints non-default values and shows at
   least one Ledger-logged autotune event.
4. For dual-DiskANN: a vault with an asymmetric slot has both `asym_a` and
   `asym_b` graph directories populated on disk.
5. For multi and concat server indexes: a synthetic >1e6-row issue #604 evidence
   root contains populated `idx/slot_00.token.ann/{graph.cda,docs.cdt,token_docs.u32,tokens.f32}`
   and `idx/xterm.concat.ann/{graph.cda,keys.cdx}`, with recall checked against
   brute-force and byte headers read back with `xxd`/`stat` on aiwonder.
6. All byte-level evidence (file sizes, p99 measurement, ANN metric-class
   readback, autotune log) attached to the PH68 GitHub issue. Grounded
   intelligence evidence, when claimed, is attached separately from Assay and
   Lodestar against a validity-audited real outcome.

## Risks / landmines

- **`hotpool` has no redundancy.** ANN graphs and posting lists are rebuildable
  from base+slots (A16 / `04 §7`); a corrupt index triggers a `degraded` flag and
  background rebuild, never data loss. Do not claim durability for the index files
  themselves — only for the base CF + WAL.
- **I/O amplification on random beams.** DiskANN beam search issues one random
  I/O per beam step; beamwidth of 64 on a cold cache = 64 seeks per query. Size
  the page-aligned block and prefetch depth so the NVMe queue depth is saturated,
  not serialized. Profile on `hotpool` before declaring the SLO met.
- **RAM footprint of SPANN centroids.** Centroid count (typically √N) for 1e9 cx
  with 15 slots must fit within the VRAM+RAM budget alongside active TEI and Forge
  matmul working sets. Measure with `calyx bench memory --vault <vault>` before
  commit.
- **Anneal oscillation.** Beamwidth/posting-cutoff autotune must have a tripwire
  (A14): if recall@10 drops below the floor, revert immediately and Ledger-log the
  revert. The Anneal soak in T06 must prove no oscillation over ≥1e5 queries.
- **`EXDEV` on ZFS temp writes.** Any tmp file produced during graph build or
  posting-list compaction must be staged inside the target dataset
  (`/zfs/hot/calyx/<vault>/`), never in `/tmp` or another dataset
  (`04 §3` / `aiwonder-system.md`).
- **GPU contention.** Distance recomputation during rescore uses Forge CUDA. The
  VRAM budgeter (PH57) must gate these dispatches so they coexist with the 3
  resident TEI containers on the RTX 5090.
- **Billion-scale embedded** is explicitly out of scope (`17 §3.4`). If any code
  path or test parametrizes over 1e8+ cx without an `#[cfg(server)]` or
  `#[ignore = "server-only"]` annotation, it is a bug.
