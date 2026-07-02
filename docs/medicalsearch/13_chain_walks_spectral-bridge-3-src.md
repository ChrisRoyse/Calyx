# 13 - chain walks spectral-bridge-3-src

- **Issue:** #880   **Phase:** P0 discovery   **Date (UTC):** 2026-07-02   **Vault/panel:** `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J`
- **Goal:** record the real-corpus grounded chain walk for seed `spectral-bridge-3-src` with terminal A-B-C provenance.

## Source artifact
- FSV root: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z`
- Report artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json`
- Report SHA256: `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd`
- Readback artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/readback_summary.json`
- Graph readback: `198993` nodes, `2435817` edges.

## Chain readback
- Seed kind: `static_candidate`
- Start node: `76cdb0f7234f9e0b25cc8fea8daf2434`
- Termination: `frontier_exhausted`
- Accepted hops: `159`
- Candidate hops inspected: `2512`
- Gate pass count: `1196`
- Refused count: `1316`
- Hypotheses emitted for this seed: `8`
- Top rank score: `0.82124996`
- Top confidence: `1.0`
- Top path length: `54`

## Terminal A-B-C
| Role | Node | source_id | source_sha256 |
|---|---|---|---|
| A | `76cdb0f7234f9e0b25cc8fea8daf2434` | `1ec3ea96-8aa8-4700-95c8-1ac7e39f952a` | `d2d8a786099cd7ba122cdc1a99d2061ae75f53a0520ce153abdc964eea770766` |
| B | `a8cc65ec3a9ae0d9febc02a22c107009` | `205bb8f3-5911-4805-a803-eaf140b3ccb3` | `d2765e2c8fadc1379a5cf6b27ef47be2cacd63d752164f7035d097d858a369a5` |
| C | `ccf62e2fb59b20a8ca50febd517f5c9b` | `e7a73ec9-3a47-48e0-b814-db25b439b4f2` | `a43c12af05fb16ad4b3b5b2dfb37b5563d2d9ea70bac5f9e6ecd492a83c33be1` |

Metadata readback for all terminal roles: `source_dataset=medmcqa`, `download_uri=hf://openlifescienceai/medmcqa`, `license=mit`, `retrieval_ts=2026-06-23T20:00:00Z`.

## Honest conclusion
This seed produced a completed grounded chain and 8 terminal hypotheses. The table records the top-ranked A-B-C terminal from the persisted artifact; all accepted-hop logs and lower-ranked hypotheses remain in the report JSON. This is a traceable hypothesis only, not a biomedical verdict.
