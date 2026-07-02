# 13 - chain walks spectral-bridge-1-src

- **Issue:** #880   **Phase:** P0 discovery   **Date (UTC):** 2026-07-02   **Vault/panel:** `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J`
- **Goal:** record the real-corpus grounded chain walk for seed `spectral-bridge-1-src` with terminal A-B-C provenance.

## Source artifact
- FSV root: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z`
- Report artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json`
- Report SHA256: `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd`
- Readback artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/readback_summary.json`
- Graph readback: `198993` nodes, `2435817` edges.

## Chain readback
- Seed kind: `static_candidate`
- Start node: `71a2dcaac4464a1943e5c17ecc5b9c4e`
- Termination: `frontier_exhausted`
- Accepted hops: `165`
- Candidate hops inspected: `2608`
- Gate pass count: `1314`
- Refused count: `1294`
- Hypotheses emitted for this seed: `8`
- Top rank score: `0.82124996`
- Top confidence: `1.0`
- Top path length: `56`

## Terminal A-B-C
| Role | Node | source_id | source_sha256 |
|---|---|---|---|
| A | `71a2dcaac4464a1943e5c17ecc5b9c4e` | `cb85e971-03f7-4a50-84d9-cf0a30ce19b9` | `37a80026910ede7fe790c47b6142f80f476407e013634ee00aaf12ccd04d8f2c` |
| B | `472cc62939d0298289886c288ea30269` | `507e291c-aa23-4aea-91f2-034a3ba6e538` | `abece60e7c8375b901cfac909fd47ca16914af719133dcf897404792b857f95a` |
| C | `d586f81bca5cf4aca40b42d2f7b65091` | `ef069ed5-8608-425a-9865-e6b30047eb45` | `1ecf092e0b207a22ec410d94b009d531302d842f96b5336abcb315fb27f47853` |

Metadata readback for all terminal roles: `source_dataset=medmcqa`, `download_uri=hf://openlifescienceai/medmcqa`, `license=mit`, `retrieval_ts=2026-06-23T20:00:00Z`.

## Honest conclusion
This seed produced a completed grounded chain and 8 terminal hypotheses. The table records the top-ranked A-B-C terminal from the persisted artifact; all accepted-hop logs and lower-ranked hypotheses remain in the report JSON. This is a traceable hypothesis only, not a biomedical verdict.
