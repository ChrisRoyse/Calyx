# #1188 Infectious / Immunology / Inflammation Association Hunt

## Scope

#1188 composes the current Calyx biomedical association substrate into a
bounded infectious disease, immunology, and inflammation evidence pack. The run
uses:

- #1183 typed all-pair hypotheses.
- #1184 falsification flags for the original typed hypotheses.
- #1171 CxId source expansion and #1172 concept normalization.
- #1174 Open Targets target-disease validation rows.
- #1178 DGIdb drug-gene rows.
- #1177 ClinicalTrials and #1181 openFDA safety context.
- Live bounded ClinicalTrials.gov and openFDA probes for selected drug-bearing
  rows.

The output is a ranked research-lead atlas. It separates antimicrobial,
antiviral, immunomodulatory, host-target, and disease-cluster rows, but it is
not a treatment, actionability, efficacy, safety, or cure claim.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1188-infectious-immunology-hunt-20260704T103019Z
```

Live query sources used during this FSV:

- ClinicalTrials.gov Data API: `https://clinicaltrials.gov/api/v2/studies`
- openFDA drug label API: `https://api.fda.gov/drug/label.json`
- openFDA drug adverse event API: `https://api.fda.gov/drug/event.json`

## Output Readback

| Artifact | Bytes | SHA-256 | Rows |
|---|---:|---|---:|
| `out/input_scope.json` | 3,591 | `38df137252ee13faefcdcd330140008b3e82ae384687c80f73133a0fc6fd0a4e` | - |
| `out/infectious_immunology_normalized_annotations.jsonl` | 403,902 | `7eac529c14b017216ea34ec05caf9445b4898517f432d07e8d13bbd7a1913bff` | 475 |
| `out/infectious_immunology_unresolved_terms.jsonl` | 69,305 | `19bc15070732f752a1fdf90552de7be4705bf8e5f236735b33fb20d5e844c90e` | 151 |
| `out/infectious_immunology_hypotheses.jsonl` | 943,059 | `de2e6eda7c8aabcb39a8644c9948923cf1b93ddf2c0776372cdbcf7f4e65b61d` | 209 |
| `out/infectious_immunology_hypotheses.json` | 1,323,474 | `67866b6d559c5537827a71f5980b319858baaf36337da5ab07f02d1b824eefab` | - |
| `out/top_evidence_bundles.json` | 100,048 | `ebe017a88af380f77e701dfb8d69e3e042b42f27f26d9ff9ebc090dcd2590c02` | 24 bundles |
| `out/external_validation_context.jsonl` | 48,980 | `4edb478a67fd6f3b13be53d45a10844a8ac3404a0cdba15011d74578ba5016dd` | 45 |
| `out/raw_query_manifest.jsonl` | 23,136 | `14fa399117e7ce09b59d58ad693167597c3fb1a7ba38208eba5392b4b5f91c0a` | 48 |
| `out/safety_trial_flags.jsonl` | 37,749 | `73116d59406dbd2d506900493048d6f37c18131458d4c8bca114143f737a5d9e` | 16 |
| `out/validation_metrics.json` | 1,720 | `d7c3dc2c5b441d884c1fbc39b7c56ec45f6434f4277329d65a7a615f1e02e104` | - |
| `out/persisted_readback.json` | - | `972ac09649ba07728509ff0efb3563e2001b3e03a7c6476584751c52df355f97` | - |

Readback assertions:

| Assertion | Value |
|---|---:|
| Hypothesis rows read back | 209 |
| Metrics hypothesis total | 209 |
| Row count matches metrics | true |
| Top bundle count | 24 |
| Raw live query rows | 48 |
| Safety/trial flag rows | 16 |
| Domain normalized annotation rows | 475 |
| Domain unresolved rows | 151 |
| Required families present | antimicrobial, antiviral, host-target, immunomodulatory |

## Metrics

| Metric | Count |
|---|---:|
| Typed rows scanned | 301 |
| Falsification flags loaded | 280 |
| Source-expanded rows loaded | 2,612 |
| Normalized annotations loaded | 2,575 |
| Infectious/immunology normalized annotations emitted | 475 |
| Infectious/immunology unresolved terms emitted | 151 |
| Open Targets rows loaded | 1,422 |
| Open Targets domain rows considered | 100 |
| DGIdb interactions loaded | 359 |
| DGIdb domain bridges considered | 486 |
| Total ranked hypotheses | 209 |
| Drug-bearing hypotheses | 120 |
| Live drug pair triage queries | 16 |
| Live API artifacts | 48 |

Hypothesis class counts:

| Class | Rows |
|---|---:|
| DGIdb drug-target/Open Targets infectious-immune bridges | 51 |
| Open Targets target-infectious-immune rows | 45 |
| Typed all-pair infectious/immunology filter rows | 93 |
| Same-source disease-to-infectious-immune clusters | 14 |
| Same-source drug/gene/variant-to-infectious-immune disease co-mentions | 6 |

Hypothesis family counts:

| Family | Rows |
|---|---:|
| Host-target | 101 |
| Immunomodulatory | 68 |
| Antiviral | 15 |
| Antimicrobial / antibacterial | 4 |
| Pathogen-disease / immune phenotype cluster | 17 |
| Infectious association | 4 |

## Top Readback Rows

| Rank | Candidate | Family | Source | Bridge | Target | Score | Evidence |
|---:|---|---|---|---|---|---:|---|
| 1 | `issue1188:dgidb_target_bridge:30b8f13223f01d85c9bf` | host-target | Golimumab | TNF | psoriatic arthritis | 1.396525469 | DGIdb + Open Targets + live safety/trial |
| 2 | `issue1188:dgidb_target_bridge:479d5510a18445362cc7` | host-target | Certolizumab Pegol | TNF | psoriatic arthritis | 1.396525469 | DGIdb + Open Targets + live safety/trial |
| 3 | `issue1188:dgidb_target_bridge:42b707a342622ba8bf02` | antiviral | Tregalizumab | CD4 | HIV infectious disease | 1.277564915 | DGIdb + Open Targets + live safety/trial |
| 4 | `issue1188:dgidb_target_bridge:1c0540ae0f79f3820461` | host-target | Placulumab | TNF | psoriatic arthritis | 1.266525469 | DGIdb + Open Targets + live safety/trial |
| 5 | `issue1188:typed:typed-assoc:concept:ncbi_gene:24835::concept:ncbi_mesh:D011507` | host-target | Tnf | - | Proteinuria | 1.25 | #1183 typed path |
| 6 | `issue1188:dgidb_target_bridge:a41ed87e4f0aa4b31e45` | host-target | Cefotaxime Sodium | TNF | psoriatic arthritis | 1.246525469 | DGIdb + Open Targets + live safety/trial |
| 7 | `issue1188:dgidb_target_bridge:2b638360564c97b39fd4` | host-target | Infliximab | IL12B | psoriasis | 1.238773092 | DGIdb + Open Targets + live safety/trial |
| 8 | `issue1188:typed:typed-assoc:concept:ncbi_mesh:D009241::concept:ncbi_mesh:D013256` | immunomodulatory | Ipratropium | - | Steroids | 1.237570627 | #1183 typed path |
| 9 | `issue1188:typed:typed-assoc:concept:ncbi_mesh:D013256::concept:ncbi_mesh:D013806` | immunomodulatory | Steroids | - | Theophylline | 1.237570627 | #1183 typed path |
| 10 | `issue1188:typed:typed-assoc:concept:ncbi_gene:920::concept:ncbi_mesh:D011507` | host-target | CD4 | - | Proteinuria | 1.204242509 | #1183 typed path |
| 11 | `issue1188:dgidb_target_bridge:1dca49f5a02e1fe7373e` | antiviral | Zanolimumab | CD4 | HIV infectious disease | 1.197287372 | DGIdb + Open Targets + live safety/trial |
| 12 | `issue1188:dgidb_target_bridge:6bf557e90eb23401a00f` | antiviral | Herbimycin | CD4 | HIV infectious disease | 1.197287372 | DGIdb + Open Targets + live safety/trial |
| 14 | `issue1188:dgidb_target_bridge:4b6698e147aa815408f8` | antiviral | Antiviral Agent | CD4 | HIV infectious disease | 1.177287372 | DGIdb + Open Targets + live safety/trial |
| 15 | `issue1188:dgidb_target_bridge:679e64b9e10db4b5fd76` | antiviral | Ibalizumab | CD4 | HIV infectious disease | 1.177287372 | DGIdb + Open Targets + live safety/trial |
| 71 | `issue1188:typed:typed-assoc:concept:ncbi_mesh:D013307::concept:ncbi_mesh:D007710` | antimicrobial | Streptomycin | - | Klebsiella Infections | 0.98248676 | #1183 typed path |
| 73 | `issue1188:normalized_comention:335220c5511f34b4eb5e` | antimicrobial | Streptomycin | - | Rhinoscleroma | 0.947258872 | source-expanded normalized co-mentions |
| 80 | `issue1188:disease_domain_cluster:8d3ae8eb6d72caa4023e` | disease cluster | Thalassemia | - | Salmonella Infections | 0.905018561 | source-expanded normalized co-mentions |

The top ranks are dominated by known-positive/calibration immunology rows:
TNF/psoriatic-arthritis and CD4/HIV target bridges validate that the pipeline
can recover established biomedical structure. They are not new treatment claims
and they currently crowd out novelty-oriented worklists, which is tracked in
#1226.

## Family Examples

| Family | Example rows |
|---|---|
| Antimicrobial / antibacterial | Streptomycin/Klebsiella Infections; Streptomycin/Rhinoscleroma |
| Antiviral | Tregalizumab/CD4/HIV infectious disease; Antiviral Agent/CD4/HIV infectious disease; Ibalizumab/CD4/HIV infectious disease |
| Immunomodulatory | Ipratropium/Steroids; Steroids/Theophylline; Prednisolone/Steroids; montelukast/Steroids |
| Host-target | Golimumab/TNF/psoriatic arthritis; Infliximab/IL12B/psoriasis; TNF/Proteinuria; CD4/Proteinuria |
| Pathogen/phenotype cluster | Rhinoscleroma/Rhinosporidiosis; Rhinosporidiosis/Klebsiella Infections; Thalassemia/Salmonella Infections |

## Falsification Status

- #1183 typed rows carry #1184 falsification status where the hypothesis id was
  present.
- Generated #1188 co-mention and DGIdb/Open Targets bridge rows were created
  after #1184 and are marked explicitly as `not_run_for_generated_*` or
  `external_validation_row_not_falsification_sweep`.
- #1223 tracks the cross-domain falsification sweep for generated disease-hunt
  candidates before atlas promotion.

## Gaps Split Out

The FSV found real follow-up work:

- #1225 - repair unresolved infectious/immunology concept normalization after
  #1188. The readback artifact contains 151 unresolved domain terms, including
  HLA-B27, bronchial asthma, acute asthma, Tuberculosis, CD40, Sepsis,
  Leukotriene, Lupus vulgaris, Malaria, and Influenza vaccine.
- #1226 - split known-positive calibration rows from novelty-prioritized
  disease-hunt rankings. #1188 recovered strong known immunology structure, but
  the global top-k is not yet optimized for novel association triage.
- #1223 - falsify generated disease-hunt candidates across domains.

## Conclusion

#1188 is complete for a bounded, persisted infectious/immunology/inflammation
association hunt:

- it produced 209 ranked rows with normalized names where available, evidence
  paths, validation context, safety/trial flags for selected drug-bearing rows,
  explicit family labels, and explicit falsification status;
- it read back all persisted output counts and hashes from aiwonder artifacts;
- it separated antimicrobial, antiviral, immunomodulatory, and host-target
  hypotheses as required;
- it exposed normalization and novelty-ranking gaps as new atomic GitHub issues.

No clinical recommendation, treatment claim, safety claim, actionability claim,
or cure claim is made by this artifact.
