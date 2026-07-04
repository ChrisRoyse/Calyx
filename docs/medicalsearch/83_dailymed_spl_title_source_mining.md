# #1242 DailyMed SPL Title Source Mining

Status: complete.

This slice continued external source mining after #1240 by rechecking the
remaining no-hit combination-candidate universe against current NLM DailyMed v2
SPL metadata pair-title search.

Clinical boundary:

```text
DailyMed SPL-title metadata evidence is source-attributed label metadata co-mention only; not label-content interpretation, efficacy, safety, pair interaction clearance, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1242-dailymed-spl-title-mining-20260704T174500Z
```

Sealed upstream input:

```text
/home/croyse/calyx/fsv/issue1240-rxnorm-combination-products-20260704T172703Z/out/candidate_rxnorm_status.jsonl
sha256: 5293210308d19be37c976d55a34813352c7f473da152be6348991e35deb829f7
```

Source contract:

- The input filter was `overall_external_source_status_after_issue1240 == no_external_hit`.
- Input rows after the filter: 1,019.
- Unique pair keys queried: 640.
- Source: NLM DailyMed v2 `/spls.json`.
- Query mode: direct pair metadata search with `drug_name`, `name_type=both`, `pagesize=100`, `page=1`.
- The source is SPL metadata title search only; it does not interpret label text or claim interaction meaning.

## Method

The miner:

- persisted official DailyMed web-service, `/spls`, about, and home pages;
- queried each pair key in both deterministic directions, `drug_a drug_b` and `drug_b drug_a`;
- required both candidate names to appear in returned SPL titles before marking a hit;
- wrote exact/normalized/no-hit status rows with raw response hashes;
- preserved all candidate rows as blocked research triage rows;
- wrote a 640-row bridge-corpus slice for native Calyx materialization.

Each `dailymed_spl_title_query_responses.jsonl` row represents one pair key
and contains both directional HTTP responses. The run made 1,280 HTTP requests,
all HTTP 200, and returned zero SPL metadata records for this candidate
universe.

## Source Artifacts

| Source artifact | Bytes | SHA-256 |
|---|---:|---|
| `raw/dailymed_web_services.html` | 85,397 | `3fbe63342062c085fcbb85f1431f6c1614bbacc6b2ade5adc21de8eba3c4327e` |
| `raw/dailymed_spls_api.html` | 91,702 | `727d6a6a7345430e54100f230ee545081f23fcffb120a78e1c047ecfdba27add` |
| `raw/dailymed_about.html` | 82,626 | `f8d463a597663bfc1b0e5aa49a566ca62d77950f2b5b132a149f088623cc9d96` |
| `raw/dailymed_home.html` | 75,305 | `4e39c0a4acdfb8e540649d1641ff366d8132002cd4e6dfa5e458969b49126ee0` |

Source URLs:

- https://dailymed.nlm.nih.gov/dailymed/app-support-web-services.cfm
- https://dailymed.nlm.nih.gov/dailymed/webservices-help/v2/spls_api.cfm
- https://dailymed.nlm.nih.gov/dailymed/about-dailymed.cfm
- https://dailymed.nlm.nih.gov/dailymed/

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `dailymed_spl_title_query_responses.jsonl` | 640 | 2,042,265 | `4fe25859869849d5172d79d11b6226eba14cd83fc26a2cf1d8bd311cc1f38ebd` |
| `dailymed_spl_title_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `dailymed_spl_title_pair_status.jsonl` | 640 | 793,368 | `64e1a84ffb4252ebac671f15e94bd3b617a69dcb0294b817852bfbec8c9004fc` |
| `candidate_dailymed_title_status.jsonl` | 1,019 | 1,378,996 | `e834783552a1b94172b1b424f15f9f6403ada53439e85a3bea67edd458f0bd26` |
| `dailymed_spl_title_bridge_rows.jsonl` | 640 | 833,585 | `87046e260c78d85eff97f5c3fd2f8da7f81c472e1b8d01f7b05ee5ebae4fb8ad` |
| `input_manifest.json` | - | 4,351 | `f33319270ca864a1aecae5f6766cf81f5c0a56fd8b17fd1ca7e7d32e3153da08` |
| `validation_metrics.json` | - | 1,297 | `812a690a8f9004f57a1f092822396a918f420842c440e1e7ca0a35d9f5dae415` |
| `output_manifest.json` | - | 2,437 | `2b3d51f75139253e78e4338f0c1691452a2297f66cf6a8e917ed29986d8da3f9` |
| `persisted_readback.json` | - | 3,674 | `4b0c05ba8190b6f46ffdf8ff9759bfa186a784cee6da58c4be9d95daaa027147` |
| `calyx_bridge_corpus_stdout.json` | - | 676 | `cbd6ff3378b2cb4050187a0822567bff2673681cd5266d8fd8cf2fdd244523fa` |
| `calyx_bridge_corpus_readback.json` | - | 3,839 | `096df2c2282176d905b7507346b88b04a7bbbb261fb6e92d323b6e7763873944` |

## Metrics

| Metric | Count |
|---|---:|
| #1240 remaining no-hit candidate rows checked | 1,019 |
| Unique pair keys queried | 640 |
| Directional HTTP requests | 1,280 |
| HTTP 200 responses | 1,280 |
| Total SPL metadata records returned | 0 |
| Verified DailyMed title evidence rows | 0 |
| Candidate rows with #1242 hit | 0 |
| Remaining no-hit candidate rows after #1242 | 1,019 |

Status counts:

| Status scope | `exact_hit` | `normalized_hit` | `no_external_hit` |
|---|---:|---:|---:|
| Pair status rows | 0 | 0 | 640 |
| Candidate status rows | 0 | 0 | 1,019 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1240 persisted readback all true | true |
| #1240 Calyx readback all true | true |
| Candidate status rows cover every remaining no-hit row | true |
| Pair status rows cover every unique pair key | true |
| Query response exists for every queryable pair key | true |
| Two directional responses per query row | true |
| All status values are allowed | true |
| All status rows carry the clinical boundary | true |
| All candidate rows remain blocked | true |
| All hits have evidence | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1242-dailymed-spl-title-mining-20260704t174500z
vault_id: 01KWQ4CB0F2XPC1N8KMR7VJ962
vault_dir: /home/croyse/calyx/vaults/01KWQ4CB0F2XPC1N8KMR7VJ962
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 640 |
| Bridge terms | 845 |
| Graph nodes | 1,485 |
| Graph edges | 5,120 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

DailyMed SPL title metadata mining produced no verified source hits for the
#1240 remaining no-hit universe. The useful result is another negative source
check: 1,019 candidate rows remain `no_external_hit`, with official source
bytes, raw API responses, hashes, status rows, and Calyx graph materialization
persisted.

Downstream work should continue source expansion from
`candidate_dailymed_title_status.jsonl`, filtered to
`overall_external_source_status_after_issue1242 == no_external_hit`.
