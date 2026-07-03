# 20 - association evidence index

- **Issue:** #867
- **Date (UTC):** 2026-07-03
- **Status:** Current evidence map after #867 was reopened for over-broad closeout.
- **Scope:** Put biomedical association-mining artifacts in one place, state what is actually proven, and define the work needed to turn graph associations into useful healing hypotheses.

## Bottom line

The corpus has real association-mining evidence, but it is not yet a usable biomedical-discovery atlas.

What exists today:

- A full clinical-QA association substrate: `198,993` nodes, about `2,435,817` edges, and `13,133,538` XTerm cross-lens agreement keys.
- Real blind-spot, domain-bridge, spectral-community, discovery-chain, chain-walk, evaluator, ranking, refusal-regrounding, and molecular-bridge artifacts.
- A ranked list of `44` traceable A-B-C hypotheses, `10` flagged for human review.
- A narrow clinical-to-molecular proof slice with `metformin`, DPP4 protein, DPP4 DNA, and BindingDB/NCBI source hashes.

What is missing:

- CxId-to-source-text expansion for every candidate and ranked hypothesis.
- Biomedical concept normalization: disease, drug, gene/protein, pathway, variant, phenotype, assay, trial, and publication IDs.
- Typed association scoring, deduplication, known-positive/known-negative validation, time-split validation, and external evidence triangulation.
- Full molecular and external knowledge-graph ingest at scale.
- Disease-focused deep hunts that produce falsifiable hypotheses, not treatment claims.

No artifact below is a clinical recommendation, cure claim, or proof of efficacy. These are hypothesis-generation outputs that require validation against source text, external biomedical databases, assays, trials, safety data, and expert review.

## Source-of-truth artifacts

| Slice | Issue | Physical evidence | Key readback |
|---|---:|---|---|
| Anchored association graph / XTerms | #870 | `corpus-anchored-869-20260625T080546Z`, vault id `01KVYX0KYVBQSGVC6N2S00FX6J` | `198,993` graph nodes; about `2,435,817` edges; `13,133,538` XTerm keys |
| Kernel grounding | #871 | `/home/croyse/calyx/fsv/issue871-kernel-build-*` and `docs/medicalsearch/04_kernel_build.md` | Grounded kernel build/recall gate for downstream miners |
| Blind-spot sweep | #875 | `/home/croyse/calyx/fsv/issue875-blind-spot-real-low-20260628T035129Z/artifact.readback.summary.json` | `128` candidates; artifact path `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J/idx/blind_spot/1782619688096-5dd11782/blind_spot_sweep.json` |
| Domain bridges | #876 | `/home/croyse/calyx/fsv/issue876-domain-bridges-20260629-030832/real_pubmedqa_medxpertqa.json` | `7` candidates for `pubmedqa` vs `medxpertqa`; report SHA256 `a9649d15c48b60c28e508633e65a66acc89945da21fcf0c9124f519ee4731f04` |
| Spectral communities | #877 | `/home/croyse/calyx/fsv/issue877-real-spectral-20260629-045547/happy2_readback_summary.json` | `2` communities; `32` bridge candidates; `32` centrality proposers; report SHA256 `4dec84d08ae12ef67908ba43f46d2a16082530d026f931e9ed11d3f5e936b4f7` |
| Full-anchor discovery chain | #878 | `/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/happy_readback_summary.json` | `198,993` anchors; `25,472` candidates; `21,936` gate passes; `1,600` accepted hops; max hop `100` |
| Probe matrix / regrounding surface | #879/#883 | `/home/croyse/calyx/fsv/issue883-real-reground-expansion-20260702T091116Z/readback_summary.json` | Before: `5` refusals, `0` grounded hits. After targeted evidence/lens addition: `5` grounded productive hits |
| Chain walks | #880 | `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json` | `6` completed seeds; `48` terminal A-B-C hypotheses; report SHA256 `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd` |
| Hypothesis evaluation | #881 | `/home/croyse/calyx/fsv/issue881-real-hypothesis-evaluation-20260702T093012Z/hypothesis_evaluation_report.json` | `48` evaluated; `44` retained; `4` rejected; report SHA256 `836a00ca7bc137194e1ea60831e4110283252fe9f17fe8e8d1ce15f49ccd470b` |
| Ranked hypotheses | #882 | `/home/croyse/calyx/fsv/issue882-real-ranked-hypotheses-20260702T100214Z/ranked_hypotheses_report.json` | `44` ranked; `10` human-review flags; report SHA256 `0483d8bc475526f65d76cd2fbb8a2a42c59751fa463c338b6e3fba54ac992257` |
| Bridge corpus | #994 | `/home/croyse/calyx/fsv/issue994-nonclinical-final-20260702T112430Z/fsv_readback.json` | `4` rows; clinical/molecular domains; `metformin` bridge; report SHA256 `fecb530aa2d5d1414f86116d57fb1cceaae17c34c4191aadc7fff0c858f9821f` |
| Molecular vault | #884 | `/home/croyse/calyx/fsv/issue884-molecular-vault-20260703T142944Z/fsv_summary.json` | `4` rows; text/molecule/protein/DNA; `13` anchors; `12` measured slots; `metformin` bridge gate pass |
| Association result pack | #1170 | `/home/croyse/calyx/fsv/issue1170-association-result-pack-20260703T154220Z/readback_summary.json` and `docs/medicalsearch/21_association_result_pack.md` | `1,949` machine-readable result rows across #875/#876/#877/#878/#880/#881/#882/#883/#884/#994; `0` missing required source refs |
| CxId source expansion | #1171 | `/home/croyse/calyx/fsv/issue1171-complete-cxid-source-expansion-20260703T153528Z/readback_summary.json` and `docs/medicalsearch/22_cxid_source_expansion.md` | `2,612` current association-result CxIds source-row verified; `0` unresolved; `8` molecular row records included; source vault not mutated |
| Biomedical concept normalization | #1172 | `/home/croyse/calyx/fsv/issue1172-biomedical-concept-normalization-20260703T154840Z/readback_summary.json` and `docs/medicalsearch/23_biomedical_concept_normalization.md` | `3,347` extracted candidate terms accounted for: `79` normalized terms and `3,268` explicit unresolved rows; `2,575` exact-span annotation rows |
| Typed biomedical overlay graph | #1173 | `/home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z/typed_graph_summary.json` and `docs/medicalsearch/24_typed_biomedical_overlay_graph.md` | `7,928` nodes; `116,753` typed edges; `0` untyped/invalid edges; top current clusters are asthma-pharmacology calibration signals |
| Open Targets validation ingest | #1174 | `/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z/readback_summary.json` and `docs/medicalsearch/25_open_targets_validation_ingest.md` | Open Targets `26.06`; `44` concept mappings attempted; `1,422` association rows; `1,420` validation edges; `23` edges map both sides to overlay concepts |
| Molecular vault scaleout | #1175 | `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/fsv_summary.json` and `docs/medicalsearch/26_molecular_vault_scaleout.md` | `53` measured ChEMBL/BindingDB/Open Targets/sequence rows; `26` molecule rows; `4` protein rows; `1` DNA row; `8` clinical-molecular bridge candidates |

## Current useful leads

These are the highest-signal outputs now expanded to source rows by #1171, but not yet normalized to biomedical concepts.

### Ranked A-B-C hypotheses

Top 10 from #882:

| Rank | Hypothesis ID | Rank score | Eval aggregate | Distance | Evidence | Human review |
|---:|---|---:|---:|---:|---:|---|
| 1 | `operator-centrality-2::01` | `0.82295454` | `0.7325` | `51` | `3` | yes |
| 2 | `operator-centrality-2::02` | `0.82295454` | `0.7325` | `51` | `3` | yes |
| 3 | `operator-centrality-2::03` | `0.82295454` | `0.7325` | `51` | `3` | yes |
| 4 | `spectral-bridge-2-src::01` | `0.82295454` | `0.75250006` | `51` | `3` | yes |
| 5 | `spectral-bridge-2-src::02` | `0.82295454` | `0.75250006` | `51` | `3` | yes |
| 6 | `spectral-bridge-2-src::03` | `0.82295454` | `0.75250006` | `51` | `3` | yes |
| 7 | `spectral-bridge-2-src::04` | `0.819318` | `0.7425` | `50` | `3` | yes |
| 8 | `spectral-bridge-2-src::05` | `0.819318` | `0.7425` | `50` | `3` | yes |
| 9 | `spectral-bridge-2-src::06` | `0.819318` | `0.7425` | `50` | `3` | yes |
| 10 | `spectral-bridge-2-src::07` | `0.809432` | `0.73375` | `49` | `3` | yes |

Representative top claim shape:

```text
operator-centrality-2: 5194ee0bbcc455a10b1b453735169a83 -- a8cc65ec3a9ae0d9febc02a22c107009 -- ccf62e2fb59b20a8ca50febd517f5c9b
```

These CxIds are now expanded in #1171. The source rows show this top cluster is a known asthma-pharmacology teaching signal, useful for calibration rather than as a new discovery claim.

### Domain bridges

#876 mined a real clinical-QA domain pair:

```text
left:  metadata:source_dataset=pubmedqa
right: metadata:source_dataset=medxpertqa
```

Top candidates:

| Rank | CxId | Source dataset | Source id | Distance | Gate confidence | Rank score |
|---:|---|---|---|---:|---:|---:|
| 1 | `0fa503037d5d87b51187abe53d1df67c` | `medmcqa` | `e0b24dc3-4133-42db-b44f-03ef01330c5b` | `2` | `0.33333334` | `0.5754386` |
| 2 | `ade680d87cf3b49a689e6381ff90c751` | `medmcqa` | `db914663-dfe1-4bc1-bb19-f54129654797` | `2` | `0.33333334` | `0.5491228` |
| 3 | `b9abb2d04ac805a4d7d3f6a1c5ccec1b` | `medmcqa` | `6486b7cd-4a08-4445-b0f9-f0b69fe1fc56` | `2` | `0.33333334` | `0.5345029` |
| 4 | `9d51f0ecb24078d47082ad964681f0ab` | `medxpertqa` | `Text-2033` | `1` | `0.5` | `0.5302631` |
| 5 | `342e4ce4d217f9b678543c1c95f90a54` | `medmcqa` | `a6d4008e-c9f4-4065-b59e-e4c97a6d3070` | `2` | `0.33333334` | `0.51988304` |

### Spectral communities

#877 partitioned the full graph into two communities:

- Community `0`: `33,871` members.
- Community `1`: `165,122` members.
- Spectral gap: `0.9430237`.
- Top bridge: `71a2dcaac4464a1943e5c17ecc5b9c4e -> 5f94d150f749709e0367ffcc4a6b2255`, rank `0.8651129`.
- Top centrality proposer: `5f94d150f749709e0367ffcc4a6b2255`, rank `0.9356725`, degree `116`.

### Blind spots

#875 readback:

- Observations scanned: `10,064,934`.
- Low alerts: `5,074,777`.
- Positive-delta rows: `5,074,777`.
- No-neighbor-evidence rows: `3,068,604`.
- Returned candidates: `128`.
- First candidate: `51deb3e383fedb2d7cafc1ee54e16c1f`, slots `8` vs `9`, delta `0.18488332629203796`, rank score `0.42941832542419434`, gate confidence `1.0`.

### Molecular bridge

#994/#884 proved the narrow clinical-to-molecular path:

- Candidate: `metformin`.
- BindingDB row `50408024` links metformin (`CHEMBL1431`) to DPP4, with PMID `18068977` and DOI `10.1016/j.bmcl.2007.11.107`.
- BindingDB row `108468`, SMILES `CN(C)C(N)=NC(N)=N`, EC50 `150000 nM`, links metformin to Streptokinase A and should not be treated as the DPP4 binding row.
- DPP4 protein row: BindingDB target FASTA SHA256 `e33decfbec34872ac376a4e08312f5cd94f3baa546d24f77dd83185a571b2c81`.
- DNA: NCBI RefSeq `NM_001935.4 Homo sapiens DPP4 mRNA`, FASTA SHA256 `024fa67def136d8572230fe1eb301de3b877c56c76a6de6015cad7fdd63fd4b7`.
- BindingDB zip SHA256 `87d69d552be6dff78bb8e071d0e6c5b4c8f98312cce354ecb0b1ab8bfa8650c7`.

This proves the bridge machinery, not a new therapy.

## What should be possible with the data

Research and existing public resources indicate the following are realistic deliverables if the current graph is normalized, typed, and validated:

- Literature-based discovery over A-B-C paths: find implicit A-C hypotheses through shared B terms, but rank/filter aggressively because unbounded B-term enumeration is noisy.
- Drug repurposing by graph path features: use explainable paths between compounds, diseases, targets, pathways, phenotypes, and publications, then score against known treatment edges.
- Target-disease prioritization: triangulate graph candidates against evidence categories and association scores from target-disease platforms.
- Molecular bridge expansion: map clinical terms to drug/target/assay rows from BindingDB, ChEMBL, Open Targets, DGIdb, and related sources.
- Transcriptomic reversal screens: compare disease signatures with perturbation signatures from Connectivity Map/LINCS-style resources, treating negative correlation as a lead, not proof.
- Cancer-specific hypothesis generation: connect cancers, genes, variants, drugs, pathways, trials, and evidence levels using precision-oncology resources, with explicit safety and evidence-level gates.
- Trial and safety triage: for every drug/disease hypothesis, read ClinicalTrials.gov status and safety/adverse-event evidence before ranking it as actionable.
- Falsification-first review: every retained hypothesis needs counter-evidence queries, contradictory literature, trial failures, toxicity flags, and known-mechanism conflicts.

Research anchors:

- Literature-based discovery / A-B-C discovery: Swanson-style association discovery and modern LBD reviews emphasize hypothesis generation, ranking, and validation rather than verdicts.
- Open Targets Platform: supports systematic target identification/prioritization and scores target-disease evidence by source/category.
- Hetionet / Project Rephetio: drug repurposing can be modeled by typed network paths between compounds and diseases.
- Connectivity Map / LINCS: disease/drug expression-signature reversal can produce drug-candidate leads, subject to reproducibility and context limits.
- PubTator3: automated biomedical entity and relation annotation can normalize text to genes, diseases, chemicals, variants, species, and cell lines.
- UMLS/MeSH: concept identifiers and biomedical vocabularies are needed to make CxId outputs comparable across sources.
- ClinicalTrials.gov: trial records can be queried programmatically for intervention/condition/status evidence.
- DGIdb and OncoKB/CIViC-like precision oncology sources: useful for drug-gene and cancer-variant actionability checks, subject to license/API constraints.

## Derived issue tree

The #867 epic should be considered complete only after these atomic tasks are closed with FSV:

1. #1170 - Build a machine-readable association result pack from all current FSV roots. **Done:** `1,949` rows in `/home/croyse/calyx/fsv/issue1170-association-result-pack-20260703T154220Z`.
2. #1171 - Expand every #875/#876/#877/#878/#880/#882 CxId to source text, metadata, and evidence path. **Done for current association-result surfaces:** `2,612/2,612` verified in `/home/croyse/calyx/fsv/issue1171-complete-cxid-source-expansion-20260703T153528Z`.
3. #1172 - Normalize every expanded evidence row to biomedical concepts. **Done as bounded first pass:** `3,347` extracted candidate terms accounted for in `/home/croyse/calyx/fsv/issue1172-biomedical-concept-normalization-20260703T154840Z`.
4. #1173 - Build a typed biomedical association overlay graph. **Done:** `7,928` nodes and `116,753` typed edges in `/home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z`.
5. #1174 - Ingest Open Targets target-disease evidence for external validation. **Done as bounded overlay validation:** `1,422` association rows and `1,420` validation edges from Open Targets `26.06`.
6. #1175 - Scale ChEMBL/BindingDB molecular vault beyond the four-row proof slice. **Done:** `53` measured rows in `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z`; runtime fixed for repeated CPU multimodal adapter rows.
7. #1176 - Ingest PubTator/PubMed relation evidence for concept-pair validation.
8. #1177 - Ingest ClinicalTrials.gov intervention-condition evidence for trial-readiness triage.
9. #1178 - Ingest DGIdb/druggable-gene evidence for drug-target triage.
10. #1179 - Add LINCS/CMap transcriptomic reversal screen for drug-repurposing hypotheses.
11. #1180 - Add precision-oncology validation sources for cancer hypotheses.
12. #1181 - Add safety/adverse-event and contraindication triage for drug hypotheses.
13. #1182 - Add known-positive/negative and time-split validation gates for association mining.
14. #1183 - Implement bounded all-pair typed association miner with deduplication.
15. #1184 - Implement counter-evidence and falsification sweep for every retained hypothesis.
16. #1185 - Run oncology deep association hunt over cancer/drug/gene/variant evidence.
17. #1186 - Run metabolic/cardiovascular drug-repurposing association hunt.
18. #1187 - Run neurodegeneration and neuropsychiatric association hunt.
19. #1188 - Run infectious/immunology/inflammation association hunt.
20. #1189 - Run rare-disease phenotype/gene/drug association hunt.
21. #1190 - Implement drug-combination and synergy hypothesis miner.
22. #1191 - Materialize CSR and traversal caches for the #869 graph and large association readers.
23. #1192 - Repair and scale large-corpus probe-matrix association search.
24. #1193 - Publish human-review biomedical hypothesis atlas.
25. #1194 - Add GPU/sparse acceleration path for broad graph association mining where warranted.

## Immediate next executable task

The next task is not another vague mining run. It is:

```text
Ingest PubTator/PubMed relation evidence for concept-pair validation (#1176).
```

The system now has target-disease validation and a measured ChEMBL/BindingDB molecular bridge slice. It still lacks relation-level literature validation, trial/safety triage, counter-evidence, and disease-area deep hunts before any hypothesis can be treated as actionable.
