# 03 - Loom weave

- **Issue:** #870   **Phase:** CPU-safe pre-corpus slice   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic XTerm CF / corpus pending #869
- **Goal:** Record and verify the Loom cross-term to Lodestar association-graph path before the anchored corpus ingest finishes.

## What was run (exact commands)

```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue870_loom_weave_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder archived-source FSV
git archive --format=tar -o issue870-20260625T123001Z-base.tar HEAD
git diff --cached --binary > issue870-20260625T123001Z.patch
ssh aiwonder "rm -rf /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z && mkdir -p /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/repo"
scp issue870-20260625T123001Z-base.tar aiwonder:/home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/repo-base.tar
scp issue870-20260625T123001Z.patch aiwonder:/home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/issue870.patch
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/repo-base.tar -C /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/repo && cd /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/repo && git init -q && git apply /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/issue870.patch"
ssh aiwonder "root=/home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z; cd \"$root/repo\" && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=\"$root\" cargo test -p calyx-lodestar --test issue870_loom_weave_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/repo && bash scripts/linecount.sh"
```

## Corpus AssocGraph design (grounded) — resolves the #870 node/edge/confidence question

The synthetic slice below exercises the mechanical path but leaves the **corpus-scale construction**
undefined (`build_assoc_graph_from_loom` needs `slot_nodes` + a `directional_confidence` per pair; the
test hand-codes both). Resolved from the operator's published theory + the LBD literature:

**Two distinct structures (both built by the corpus weave):**

1. **Within-doc DDA cross-terms (the literal "Loom weave").** For each constellation, the C(N,2)
   cross-lens **agreement** scalars across the (≤14) content lenses = the doc's *definition* /
   constellation signature (*Calculus of Association* §2.5: "meaning is the cross-space binding";
   differentiation contract: ≥0.05 bits/instrument, no pair corr >0.6). Materialized eagerly into the
   **XTerm CF** via `LoomStore::weave` → `persist_xterms_to_aster`. Satisfies "cross-term CF populated".
   Also yields the **blind-spot** signal (cross-lens disagreement = novelty) used downstream (#875).

2. **Between-doc directed AssocGraph (the discovery/kernel graph).** Per *The Oracle and the Kernel*
   §4.3: **nodes = constellations (artifacts); directed edge `X ← Y` ("X is definable from Y by
   association") when, given grounded `Y`, the panel predicts `X` above a confidence threshold —
   a constellation-membership test in representation space.** Operationally for the corpus:
   - **Edge candidates:** nearest neighbors of `X` in the **fused panel representation** (the
     RRF/search index already built for the vault) — kNN, `k` bounded.
   - **Directional confidence `conf(X←Y) ∈ [0,1]`:** asymmetric panel-prediction confidence that
     grounded `Y` predicts `X` (membership cosine, weighted by `Y`'s rank in `X`'s neighbor list).
     Asymmetric by construction (kNN is asymmetric) — matches the field's **asymmetric-transitivity**
     requirement for LBD graph embedding (Alzheimer's LBD link-prediction, ScienceDirect S1532046423001855).
   - **Node weight = groundedness:** anchored nodes (QA-label anchors from #869) weight 1.0;
     `groundedness_fraction` = fraction of nodes that reach an anchor within `max_groundedness_distance`.
   - Edge weight = `agreement_or_membership × directional_confidence` (`calyx_mincut` graph builder).

   This is the graph the **kernel (#871)** runs **SCC → Brandes betweenness → top-fraction kernel →
   MFVS (≈1% Minimum Grounding Set)** over — the MFVS = *minimum feedback vertex set*
   (Vincent-Lamarre et al. 2016, *The Latent Structure of Dictionaries*; NP-hard, Karp 1972).

**Implementation:** new `calyx weave-loom <vault>` CLI command (mirrors `rebuild-search-index`):
iterate Base CF → weave within-doc cross-terms into XTerm CF → build the between-doc directed AssocGraph
(nodes=constellations, asymmetric kNN edges in fused space, anchor-grounded node weights) → emit
node/edge/provenance/unique-xterm/groundedness_fraction → record here. Acceptance: XTerm CF populated,
edge/node counts recorded, `groundedness_fraction > 0`.

**Sources:** *The Oracle and the Kernel* §4.3; *The Calculus of Association* §2.3/§2.5 (cross-terms,
cross-space binding, differentiation contract); Vincent-Lamarre et al. 2016 *Topics in Cognitive Science*
(MinSet = MFVS ≈1%); graph-embedding LBD with asymmetric transitivity (ScienceDirect S1532046423001855).
Validated 2026-06-27.

## Raw evidence / FSV

Implementation source:
- `crates/calyx-lodestar/src/loom_weave_report.rs`
- `crates/calyx-lodestar/tests/issue870_loom_weave_tests.rs`
- `crates/calyx-lodestar/src/lib.rs`

The report consumes the existing `build_assoc_graph_from_loom` adapter output. It records:
- `node_count`
- `edge_count`
- `provenance_count`
- `unique_xterm_count`
- `anchor_count`
- `grounded_node_count`
- `groundedness_fraction`
- `gate_passed`
- `graph_density`
- bounded `top_edges`

The synthetic FSV path writes an XTerm row through `LoomStore::persist_xterms_to_aster`, reopens the `XTerm` CF through `CfRouter`, reloads through `LoomStore::load_xterms_from_aster`, builds the Lodestar `AssocGraph`, and then writes a JSON readback artifact.

Expected scalar leaves from the happy readback:
- `persisted_xterms=1`
- `cf_row_count=1`
- `report.node_count=2`
- `report.edge_count=2`
- `report.provenance_count=2`
- `report.unique_xterm_count=1`
- `report.grounded_node_count=2`
- `report.groundedness_fraction=1.0`
- `report.gate_passed=true`

Boundary and edge behavior covered:
- No anchors records `groundedness_fraction=0.0` and `gate_passed=false`.
- Empty graph fails closed with `CALYX_KERNEL_EMPTY_GRAPH`.
- Invalid `min_groundedness_fraction` fails closed with `CALYX_KERNEL_INVALID_PARAMS`.
- Invalid `max_top_edges` fails closed with `CALYX_KERNEL_INVALID_PARAMS`.

aiwonder archived-source FSV:
- FSV root: `/home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z`
- Patch bytes: `17374`
- Base archive bytes: `28733440`
- Happy artifact: `/home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/happy/issue870_loom_weave_readback.json`
- Happy artifact bytes: `1123`
- Happy artifact SHA256: `9e4f5c18f571f67fff914d733ca0136084c6666dfcccbe5552a15c071bb3519a`
- Happy scalar leaves: `persisted_xterms=1`, `cf_row_count=1`, `schema_version=1`, `node_count=2`, `edge_count=2`, `provenance_count=2`, `unique_xterm_count=1`, `anchor_count=1`, `grounded_node_count=2`, `groundedness_fraction=1.0`, `gate_passed=true`, `graph_density=1.0`, `top_edge_edge_weight=0.800000011920929`
- Ungrounded artifact: `/home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/edges/issue870_loom_weave_ungrounded.json`
- Ungrounded artifact bytes: `124`
- Ungrounded artifact SHA256: `9af8f43b41e96952805389e47fddf6e4ab6a281495415c7e03f43341a1607b0d`
- Ungrounded scalar leaves: `node_count=2`, `edge_count=1`, `grounded_node_count=0`, `groundedness_fraction=0.0`, `gate_passed=false`
- Error artifact: `/home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/edges/issue870_loom_weave_errors.json`
- Error artifact bytes: `146`
- Error artifact SHA256: `15b46b67acc70b8ca0544edec1cfc293f0929b9c3176c443a485c028f550f556`
- Error scalar leaves: `empty_graph=CALYX_KERNEL_EMPTY_GRAPH`, `bad_fraction=CALYX_KERNEL_INVALID_PARAMS`, `bad_top_edges=CALYX_KERNEL_INVALID_PARAMS`
- aiwonder tests: 3 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder final live-checkout FSV after dev push:
- Dev commit: `66e79f59`
- FSV root: `/home/croyse/calyx/fsv/issue870-loom-weave-final-20260625T123500Z`
- Happy artifact: `/home/croyse/calyx/fsv/issue870-loom-weave-final-20260625T123500Z/happy/issue870_loom_weave_readback.json`
- Happy artifact bytes: `1123`
- Happy artifact SHA256: `9e4f5c18f571f67fff914d733ca0136084c6666dfcccbe5552a15c071bb3519a`
- Happy scalar leaves: `persisted_xterms=1`, `cf_row_count=1`, `schema_version=1`, `node_count=2`, `edge_count=2`, `provenance_count=2`, `unique_xterm_count=1`, `anchor_count=1`, `grounded_node_count=2`, `groundedness_fraction=1.0`, `gate_passed=true`, `graph_density=1.0`, `top_edge_edge_weight=0.800000011920929`
- Ungrounded artifact: `/home/croyse/calyx/fsv/issue870-loom-weave-final-20260625T123500Z/edges/issue870_loom_weave_ungrounded.json`
- Ungrounded artifact bytes: `124`
- Ungrounded artifact SHA256: `9af8f43b41e96952805389e47fddf6e4ab6a281495415c7e03f43341a1607b0d`
- Ungrounded scalar leaves: `node_count=2`, `edge_count=1`, `grounded_node_count=0`, `groundedness_fraction=0.0`, `gate_passed=false`
- Error artifact: `/home/croyse/calyx/fsv/issue870-loom-weave-final-20260625T123500Z/edges/issue870_loom_weave_errors.json`
- Error artifact bytes: `146`
- Error artifact SHA256: `15b46b67acc70b8ca0544edec1cfc293f0929b9c3176c443a485c028f550f556`
- Error scalar leaves: `empty_graph=CALYX_KERNEL_EMPTY_GRAPH`, `bad_fraction=CALYX_KERNEL_INVALID_PARAMS`, `bad_top_edges=CALYX_KERNEL_INVALID_PARAMS`
- aiwonder live tests: 3 passed, 0 failed, 0 ignored.
- aiwonder live `cargo fmt --all -- --check`: exit 0.
- aiwonder live `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

## Findings (honest)

- The existing Loom adapter can populate a Lodestar association graph from XTerm agreement rows and directional confidence rows.
- This CPU-safe slice now makes #870 acceptance-style metrics durable and bounded for issue-state readback.
- This is not final #870 acceptance. The real anchored corpus XTerm CF is still blocked on #869 finishing the anchored ingest, and pair-gain promotion across the full 14-lens corpus has not been proven.

## Conclusion & next step

Use this report after #869 completes to record the real corpus cross-term CF counts, agreement graph node/edge counts, and `groundedness_fraction > 0` from the live Calyx source-of-truth bytes.

---

# 03b — `calyx weave-loom` corpus implementation (2026-06-27)

The grounded design above is now **implemented** as a CLI command and verified end-to-end with
full-state verification against the real Calyx column families. Branch `issue870-loom-weave`.

## What was built

`calyx weave-loom <vault> [--content-slot <u16>] [--knn <n>] [--edge-cos-threshold <0..1>]
[--max-groundedness-distance <n>] [--batch <n>] [--limit <n>]` — a single streaming command that
populates two CFs and emits the acceptance report. New/changed source:

- `crates/calyx-cli/src/cmd/weave.rs` — entry, args/parse, orchestration, JSON report + FSV readback.
- `crates/calyx-cli/src/cmd/weave/passes.rs` — Pass A (within-doc weave + graph nodes) and Pass B
  (between-doc DiskANN k-NN edges), both streamed/batched and fail-closed.
- `crates/calyx-loom/src/agreement_graph.rs` — `LoomStore::xterm_kv_rows()` (persist XTerm through the
  vault WAL/MVCC `write_cf_batch`, not a raw `CfRouter`, so the on-disk encoding round-trips).
- `crates/calyx-lodestar/src/corpus_weave_report.rs` — pure, tested `corpus_weave_report()` measuring
  node/edge/density and **anchor-grounded `groundedness_fraction` + gate** over the between-doc graph.

### Algorithm (both structures, one Base-CF scan + one DiskANN pass)

1. **Within-doc agreement → XTerm CF.** Per constellation, the content lenses (panel slots with
   `state=Active && !retrieval_only`) are grouped by vector dimension — cosine agreement is only
   defined between equal-dimension lenses — and `LoomStore::weave` materializes the C(n,2) agreement
   scalars per dimension group into the XTerm CF (batched). The corpus 14 content lenses split
   768×12 / 384×1 / 256×1, so the 768-group yields **C(12,2)=66** agreement pairs/constellation;
   the singleton-dim lenses contribute none (recorded honestly, not silently dropped).
2. **Between-doc directed k-NN AssocGraph → `graph` CF (`PlainGraph`).** Node props = the content-slot
   embedding + anchor kinds + metadata; directed edges from the **persisted DiskANN index**
   (`PersistedSearchIndexes::search`, O(N·log N)) — top-k neighbours with cosine ≥ threshold. This is
   the graph the kernel (#871) consumes via `AsterAssocSnapshot`/`summarize_vault_latest`. The
   topology deliberately matches what `vault_kernel::build_vault_kernel_inputs` would produce
   (same content slot, threshold, top-k) but built scalably — the existing path is brute-force
   O(N²) and intractable at 199k (filed as #943).

Root cause of the prior gap: the corpus-scale construction (node assignment + directional confidence)
was undefined in code; the per-(cx,slot) `build_assoc_graph_from_loom` adapter does **not** model the
§4.3 between-constellation graph, so the command builds the between-doc graph directly via
`AssocGraph::builder()` + DiskANN k-NN.

## Synthetic full-state verification (CPU, deterministic — 2026-06-27)

Isolated vault `fsv` (`CALYX_HOME=/tmp/weave-fsv3`, vault
`01KW5A8EE9E4XSY6T0617Z4VXT`), text-default panel + 3 `algorithmic` Dense(16) lenses (slots 8/9/10).
Six synthetic docs in two token-clusters (aspirin/heart × 3, photosynthesis × 3); **4 anchored**
(`label:answer=yes` on C1,C2,C4,C5), 2 unanchored (C3,C6). `rebuild-search-index`, then
`weave-loom fsv --content-slot 8 --knn 5 --edge-cos-threshold 0.5`.

Command report (stdout): `constellations_processed=6`, `xterm.rows_persisted=18`,
`xterm.slot_pairs=[(8,9),(8,10),(9,10)]` each `n=6` `mean_agreement=1.0`,
`assoc_graph.edges_persisted=30`, `report.node_count=6`, `edge_count=30`, `anchor_count=4`,
`grounded_node_count=6`, `groundedness_fraction=1.0`, `gate_passed=true`, `graph_density=1.0`.

**Source-of-truth readback (not the return value — the actual CF bytes via `calyx readback`):**
- XTerm CF: **18 distinct keys** (= 6 docs × 3 lens-pairs); sample key `…0008000902` decodes to
  `{a:8,b:9,kind:agreement,value.scalar:0.99999994,tag:derived}`. ✓ matches `rows_persisted=18`.
- `graph` CF: **6 node keys** + **60 edge keys** (= 30 directed edges × out+in rows). ✓ matches
  `node_count=6`, `edge_count=30`.
- Each node-row value decodes to `AsterAssocNodeProps` with a **16-dim embedding** and the correct
  anchors: **exactly 4 nodes carry `anchors:[{label:answer}]`, 2 carry none** — matching the 4 docs
  anchored vs 2 left unanchored. ✓

**Boundary / edge-case audit (all fail-closed):**
- `--content-slot 0` (an Active lens with no materialized vector) → `CALYX_KERNEL_INVALID_PARAMS`
  naming the constellation + slot.
- `--content-slot 99` (not a content lens) → `CALYX_CLI_USAGE_ERROR` listing the valid slots.
- empty vault → fails closed (`CALYX_STALE_DERIVED`: search-index manifest missing — rebuild first).
- Unit tests: `corpus_weave_report` (4: groundedness, distance cap, no-anchor zero, empty/bad-params),
  `xterm_kv_rows_match_router_persist_encoding` (1), weave parse + round-trip (9). All pass on aiwonder.

## Real corpus run (#869 vault — FSV recorded 2026-06-27)

Ran `weave-loom corpus-anchored-869-20260625T080546Z` (defaults: content slots 8–21, knn_slot 8,
knn 16, edge-cos-threshold 0.5, max_groundedness_distance 3) on the 198,993-constellation anchored
corpus (`CALYX_HOME=/home/croyse/calyx`, vault `01KVYX0KYVBQSGVC6N2S00FX6J`).

**Full-state verification from the live CF bytes (`calyx readback`, distinct-key counts — not the
return value):**
- **`graph` CF: 198,993 node keys** (exactly one node per constellation) **+ 4,871,633 edge-rows →
  ~2,435,816 directed k-NN edges** (each edge = out-key + in-key). Avg out-degree ≈ 12.2 (≤ knn 16;
  neighbours below cosine 0.5 are dropped).
- **`XTerm` CF populated** (9.9 GB on disk pre-compaction): within-doc agreement cross-terms over the
  12 same-dimension (768-d) content lenses → C(12,2)=66 pairs/constellation; the 384-d (slot 18) and
  256-d (slot 21) lenses are singletons and contribute no pairs (recorded, not silently dropped).
  **Logical distinct-key count = 13,133,538 = exactly 198,993 × 66** — i.e. every constellation
  materialized all 12 of the 768-d lenses (no Absent), the clean expected count.
- **`groundedness_fraction = 1.0`** — every constellation carries QA anchors (`Label("answer")`,
  `Label("dataset")` from #869), so every node is grounded at distance 0; gate passed.

Node props verified earlier (synthetic) carry the content-slot embedding + anchor kinds, so the kernel
(#871) can read this graph via `AsterAssocSnapshot`.

**Perf**: the corpus-scale read path was rewritten mid-run from per-doc `vault.get` (random reads across
17 slot CFs — intractable, ~16 h projected) to **sequential bulk scans** (one Base scan + one scan per
content-slot CF). The acceptance report's groundedness loop was likewise rewritten from O(N²)
(`anchors.contains` per node on a fully-anchored corpus) to O(N) via a `HashSet`. Re-running into
already-populated CFs is pathologically slow (LSM compaction thrash) — run once into the empty CFs, or
clear them first.
