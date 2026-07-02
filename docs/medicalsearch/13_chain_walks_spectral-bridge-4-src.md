# 13 - chain walks spectral-bridge-4-src

- **Issue:** #880   **Phase:** P0 discovery   **Date (UTC):** 2026-07-02   **Vault/panel:** `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J`
- **Goal:** record the real-corpus grounded chain walk for seed `spectral-bridge-4-src` with terminal A-B-C provenance.

## Source artifact
- FSV root: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z`
- Report artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json`
- Report SHA256: `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd`
- Readback artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/readback_summary.json`
- Graph readback: `198993` nodes, `2435817` edges.

## Chain readback
- Seed kind: `static_candidate`
- Start node: `2b597f47ce5d9a4101918d06272cb294`
- Termination: `frontier_exhausted`
- Accepted hops: `135`
- Candidate hops inspected: `2128`
- Gate pass count: `1038`
- Refused count: `1090`
- Hypotheses emitted for this seed: `8`
- Top rank score: `0.82124996`
- Top confidence: `1.0`
- Top path length: `46`

## Terminal A-B-C
| Role | Node | source_id | source_sha256 |
|---|---|---|---|
| A | `2b597f47ce5d9a4101918d06272cb294` | `1b059bd2-20df-4b23-bdc6-21b5fb27e678` | `16c9f3af486bd951b25052c85f4be31d6032437ad67dddce41deab3c14cd258c` |
| B | `a8cc65ec3a9ae0d9febc02a22c107009` | `205bb8f3-5911-4805-a803-eaf140b3ccb3` | `d2765e2c8fadc1379a5cf6b27ef47be2cacd63d752164f7035d097d858a369a5` |
| C | `ccf62e2fb59b20a8ca50febd517f5c9b` | `e7a73ec9-3a47-48e0-b814-db25b439b4f2` | `a43c12af05fb16ad4b3b5b2dfb37b5563d2d9ea70bac5f9e6ecd492a83c33be1` |

Metadata readback for all terminal roles: `source_dataset=medmcqa`, `download_uri=hf://openlifescienceai/medmcqa`, `license=mit`, `retrieval_ts=2026-06-23T20:00:00Z`.

## Honest conclusion
This seed produced a completed grounded chain and 8 terminal hypotheses. The table records the top-ranked A-B-C terminal from the persisted artifact; all accepted-hop logs and lower-ranked hypotheses remain in the report JSON. This is a traceable hypothesis only, not a biomedical verdict.
