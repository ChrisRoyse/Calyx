# PH69 · T05 — Graph/kernel corpus acquisition (WordNet / ConceptNet / Wiktionary / Cora / ogbn)

| Field | Value |
|---|---|
| **Phase** | PH69 — Dataset acquisition + MANIFEST + checksum FSV |
| **Stage** | S18 — Datasets & Intelligence FSV |
| **Crate** | `—` (scripts/infra) |
| **Files** | `scripts/acquire_graph_kernel.sh` (≤500) |
| **Depends on** | T01 (MANIFEST schema + verify tooling) |
| **Axioms** | A2, A34 |
| **PRD** | `28 §3` row 4, `28 §3.2` |

## Goal

Acquire the graph/kernel corpora (WordNet, ConceptNet, Wiktionary definition
graph, Cora citation graph, ogbn citation graph) to
`/zfs/archive/calyx/datasets/<name>/`, checksum-verify each, and write MANIFEST
rows. These provide the real, grounded graphs that PH70 uses to prove Lodestar
kernel-only recall ≥0.95·full on ≥3 corpora (PRD `28 §2`, `28 §3` row 4).

## Build (checklist of concrete, code-level steps)

- [ ] `scripts/acquire_graph_kernel.sh`:
      WordNet — via NLTK (`python -m nltk.downloader wordnet`) to
      `/zfs/archive/calyx/datasets/wordnet/`; record `synset_count` as rows.
      ConceptNet — download `conceptnet-assertions-5.7.0.csv.gz` from
      `https://s3.amazonaws.com/conceptnet/downloads/2019/edges/conceptnet-assertions-5.7.0.csv.gz`
      (free academic mirror) to `/zfs/archive/calyx/datasets/conceptnet/`; ~34 M edges.
      Wiktionary definition graph — derive from the Wiktionary dumps (Wikimedia
      free license) or HF `wikimedia/wikipedia` definitions subset; save edge list
      to `/zfs/archive/calyx/datasets/wiktionary_defn_graph/`.
      Cora — via PyG / HF `datasets` `cora` to
      `/zfs/archive/calyx/datasets/cora/`; 2708 nodes, 5429 edges.
      ogbn-papers100M or ogbn-arxiv (smaller) — via OGB Python package to
      `/zfs/archive/calyx/datasets/ogbn/`; record split used.
- [ ] For each: record expected node/edge counts + sha256 pre-download; post-download
      verify; fail-closed on mismatch.
- [ ] MANIFEST rows for each, noting `what_it_tests = Lodestar kernel-only recall ≥0.95`.
- [ ] Ensure at least 3 of these 5 corpora are fully verified so the "≥3 corpora"
      gate for PH70 is satisfiable from this card.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: parse a synthetic 5-node, 7-edge adjacency list (fixed content, known
      sha256); assert node count = 5, edge count = 7.
- [ ] proptest: property that verify round-trips on any well-formed edge-list file.
- [ ] edge (≥3):
      (1) edge-list file with self-loop → parser records it, does not crash;
      (2) malformed line (missing second node) → `CALYX_DATASET_SCHEMA_MISMATCH`,
          exits 1;
      (3) Cora row count ≠ 2708 (partial download) → `CALYX_DATASET_ROWCOUNT_MISMATCH`.
- [ ] fail-closed: missing `HF_HUB_TOKEN` (for HF-sourced graphs) → exits 1,
      `CALYX_SECRET_MISSING: HF_HUB_TOKEN`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/zfs/archive/calyx/datasets/wordnet/`, `conceptnet/`, `wiktionary_defn_graph/`,
  `cora/`, `ogbn/` on aiwonder; MANIFEST rows.
- **Readback:**
  ```
  bash scripts/verify_dataset.sh wordnet
  bash scripts/verify_dataset.sh conceptnet
  bash scripts/verify_dataset.sh cora
  bash scripts/verify_dataset.sh ogbn
  cat $CALYX_HOME/datasets/MANIFEST.md | grep -E 'wordnet|conceptnet|wiktionary|cora|ogbn'
  python3 -c "import pathlib; lines=pathlib.Path('/zfs/archive/calyx/datasets/cora/edges.txt').read_text().splitlines(); print('edges:', len(lines))"
  ```
- **Prove:** before: directories absent; after: verify exits 0 for ≥3 of the 5;
  Cora edge count confirmed by Python one-liner; MANIFEST rows populated with sha256;
  live sha256 matches stored value.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH69 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
