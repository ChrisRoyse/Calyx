# 02 — Anchored re-ingest of the ~199k clinical-QA corpus

- **Issue:** #869   **Phase:** 0   **Date (UTC):** 2026-06-27   **Vault/panel:** corpus-anchored-869-20260625T080546Z / biomed-clinical-fast
- **Goal:** Re-ingest pubmedqa + medxpertqa + medqa + medmcqa through `biomed-clinical-fast` (batch=4) WITH anchors threaded at ingest; verify-chain ok; anchor counts match row counts.

## What was run (exact commands)
```
# aiwonder, CALYX_HOME=/home/croyse/calyx, binary /home/croyse/calyx/repo/target/release/calyx
# env: source /home/croyse/calyx/.env ; export CALYX_MEASURE_BATCH=4 ; cd /home/croyse/calyx/repo

# original launcher: fsv/issue869-anchored-reingest-20260625T080546Z/run_issue869_anchored_ingest.sh
calyx create-vault corpus-anchored-869-20260625T080546Z          # ULID 01KVYX0KYVBQSGVC6N2S00FX6J
calyx panel template swap --template biomed-clinical-fast --vault corpus-anchored-869-20260625T080546Z   # 17 active slots
for ds in pubmedqa medxpertqa medqa medmcqa; do
  calyx ingest corpus-anchored-869-20260625T080546Z --batch $OUTDIR/$ds.anchored.jsonl --idempotent
  calyx verify-chain corpus-anchored-869-20260625T080546Z
done

# FAILURE on medmcqa at 2026-06-26T01:04:40Z (see Raw evidence). Dedup + resume:
#   build medmcqa.remainder.jsonl = global text-dedup, rows after the 85,300 committed
# resume launcher: fsv/.../resume_issue869_anchored_ingest.sh
calyx ingest corpus-anchored-869-20260625T080546Z --batch $OUTDIR/medmcqa.remainder.jsonl --idempotent
calyx verify-chain corpus-anchored-869-20260625T080546Z
```

## Raw evidence / FSV

**Per-dataset ingest (original run, chain ok after each):**
```
pubmedqa    ingested=1000   elapsed=342s   rate=175rpm  chain=ok verify_rc=0
medxpertqa  ingested=2455   elapsed=860s   rate=171rpm  chain=ok verify_rc=0
medqa       ingested=12723  elapsed=5817s  rate=131rpm  chain=ok verify_rc=0
medmcqa     FAILED rc=2 elapsed=54089 first_err=
  {"code":"CALYX_ASTER_CORRUPT_SHARD","message":"CxId collision or non-idempotent duplicate constellation",
   "remediation":"restore from restic/snapshot"}   # at committed=85,300 (=21,325x4, clean batch boundary)
```

**Root cause (corpus data issue, not a Calyx storage bug):** `cx_id = blake3(text, panel_version, vault_salt)` — text only (`crates/calyx-core/src/ids.rs:195`); metadata is stored on the constellation base but not hashed. `medmcqa.anchored.jsonl` (182,822 rows) had **7 duplicate question texts** with differing `source_id`, so the second occurrence collides on cx_id with a different base and Aster fails closed (`crates/calyx-aster/src/vault/anchor_merge.rs:13`). First collision = input line 85,301 (exactly where it died). The guard fires **before commit**, so the vault stayed intact — `verify-chain` on the partial vault returned `{"status":"ok","checked":330133,"break_at":null}`.

**Dedup:** global text-dedup (keep first occurrence) dropped exactly 7 lines: `85301, 90442, 104838, 128549, 135132, 146279, 171591`. Remainder = `182822 - 85300 - 7 = 97,515` rows (`medmcqa.remainder.jsonl`, sha256 `fd3b34aa6ea252e4de336f32311e60182394e3d023e035dda39e321eebb36f34`).

**Resume:** all 97,515 remainder rows committed (no further collisions). Steady-state rate ~45–76 rows/min over ~28 h (gentle decline as vault grew).

**Finalization hang (separate Calyx bug, found + fixed):** after the last row committed and the Aster manifest/CURRENT sealed (14:49:19Z, seq 99500), the `calyx ingest` process did not exit — one thread spun ~100% userland (`utime` +6001 ticks/60s, `stime`=0), `read_bytes`/`write_bytes` deltas both 0, state `R`, `wchan` 0, RSS ~30 GB, ~38 min until killed. Root cause: post-commit search-index rebuild serialized the full ColBERT multi-vector sidecar via `serde_json::to_vec_pretty` into one multi-GB buffer + serial sha256 (`crates/calyx-search/src/persisted/{multi,sparse,filter}.rs`). Data was already durable, so: `kill -TERM` (exited rc=143) → independent `calyx verify-chain` →
```
{"status":"ok","checked":647374,"break_at":null}
```
Fix: stream compact JSON via `serde_json::to_writer` into a hashing `BufWriter` (`write_json_atomic_hashed` in `crates/calyx-search/src/persisted.rs`) — no full-buffer materialization, hash folded into the single write pass.

## Findings (honest)
- **Acceptance met:** fresh anchored vault ✅; `verify-chain status:ok` (647,374 ledger entries, no break) ✅; constellation count **198,993** = 199,000 − 7 deduped (pubmedqa 1,000 + medxpertqa 2,455 + medqa 12,723 + medmcqa 182,815) ✅. Anchors were threaded at ingest from each row's `anchors[]` (label:answer, label:dataset, test-pass), same code path as `calyx anchor`.
- **Two distinct defects surfaced:** (1) a corpus data-quality issue — the anchored-input builder must dedup by `text` at generation time (same text + different `source_id` ⇒ cx_id collision); (2) a real Calyx bug — `to_vec_pretty` serialization of large multi-vector sidecars stalls the ingest close path (fixed here).
- **Caveat — search indexes not yet rebuilt:** the hang was killed *during* the search-index rebuild, so the vault's `idx/search` sidecars are incomplete for this vault. The ledger/data is complete and verified; the sidecars must be rebuilt with the fixed binary before search/discovery use.

## Conclusion & next step
The 198,993-constellation anchored corpus is durably ingested and chain-verified — this unblocks the downstream discovery work (Loom weave #870, kernel build #871). Next: (1) land the finalization-hang fix (PR) and update the aiwonder runner binary; (2) rebuild this vault's search-index sidecars with the fixed binary (now fast); (3) fix the anchored-input builder to dedup by text so re-ingests can't recollide.
