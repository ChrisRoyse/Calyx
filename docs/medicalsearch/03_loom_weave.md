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
