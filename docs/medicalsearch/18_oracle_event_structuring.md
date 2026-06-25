# 18 - Oracle event/domain structuring

- **Issue:** #885   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic structured QA / algorithmic test panel
- **Goal:** Thread QA rows into Oracle metadata plus Recurrence CF context so `reverse_query` can return grounded causes from a structured corpus.

## What was run (exact commands)

aiwonder source tree: `/home/croyse/calyx/repo`
FSV root: `/home/croyse/calyx/fsv/issue885-oracle-event-20260625T103025Z`

```bash
cd /home/croyse/calyx/repo

cargo fmt --all -- --check \
  >"$FSV_ROOT/fmt.stdout" 2>"$FSV_ROOT/fmt.stderr"
git diff --cached --check \
  >"$FSV_ROOT/diff_cached.stdout" 2>"$FSV_ROOT/diff_cached.stderr"
bash scripts/linecount.sh \
  >"$FSV_ROOT/linecount.stdout" 2>"$FSV_ROOT/linecount.stderr"

CALYX_FSV_ROOT="$FSV_ROOT" \
  cargo test -p calyx-cli cmd::ingest::oracle_event_tests -- --nocapture \
  >"$FSV_ROOT/oracle_tests.stdout" 2>"$FSV_ROOT/oracle_tests.stderr"

cargo test -p calyx-aster durable_vault_writes_wal_sst_manifest_and_cold_opens -- --nocapture \
  >"$FSV_ROOT/durable_regression.stdout" 2>"$FSV_ROOT/durable_regression.stderr"

cargo build -p calyx-cli \
  >"$FSV_ROOT/cli_build.stdout" 2>"$FSV_ROOT/cli_build.stderr"

target/debug/calyx readback recurrence-series --vault "$vault" --cx-id "$cx" \
  >"$FSV_ROOT/recurrence_readback.stdout" 2>"$FSV_ROOT/recurrence_readback.stderr"
```

## Raw evidence / FSV

Bounded aiwonder readback:

- `fmt_rc=0`, stdout/stderr bytes `0/0`, SHA256 both `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`
- `diff_cached_rc=0`, stdout/stderr bytes `0/0`, SHA256 both `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`
- `linecount_rc=0`, stdout bytes `26`, SHA256 `2ca9608a7e23755e5f4038d3d0e6ae4482f4acf00be03489434b68af163e79f1`
- `oracle_tests_rc=0`, stdout bytes `4240`, SHA256 `83d12fdcb5d31bca836e1eecf2c5a3529a8d393afc894113d5d1490601403fec`; stderr bytes `4486`, SHA256 `f4346e9f429a17b4921df2990a0dc1a60a714bbf44a914e73911f7488f94d189`
- `durable_regression_rc=0`, stdout bytes `4261`, SHA256 `5723178ddd57f47b24319da5b2db00644fabf3790f20e63f86073bfee6738449`; stderr bytes `3969`, SHA256 `ca6ad21fc9cd94def1c21ce36b29180f35c9d6fd0d09e81eab71e1e2e5369580`
- `cli_build_rc=0`, stderr bytes `1077`, SHA256 `f8f7c33e09285ecc522a4fcd5012f6e62fb6e8fe59aab6e56bd65114c9b77038`

Readback artifact:

- Path: `/home/croyse/calyx/fsv/issue885-oracle-event-20260625T103025Z/issue885_oracle_event_readback.json`
- Bytes: `859`
- SHA256: `47e86ebea61f2646aa7232b56a8189aca416cee70435378790df08c89e2a994d`
- Preserved vault: `/home/croyse/calyx/fsv/issue885-oracle-event-20260625T103025Z/calyx-cli-ingest-oracle-oracle-event-3249476-1782383453238/vaults/01KVZ5A91PF9QQNYNDP31HHJV1`
- `cx_id=0fbaa052c858412fe27ae5a97d3502ae`

Source-of-truth scalar readback from the persisted vault:

- Base metadata: `oracle.domain=endocrinology`, `oracle.action=What treats type 2 diabetes?`, `oracle.structured=true`
- Recurrence CF: `cf_rows=1`, `occurrences=1`, `first_t_secs=1700000000`
- Oracle reverse query: `cause_count=1`, `first_action_or_event=What treats type 2 diabetes?`, `first_domain=endocrinology`, `first_provisional=false`, `ledger_seq=3`
- CLI recurrence readback: stdout bytes `1126`, SHA256 `d5b941302bd2b8e90a6a7626a5578c901a41f9ea1bbf6514e490352b2d953012`, `frequency=1`, `occurrences_len=1`, `cx_id=0fbaa052c858412fe27ae5a97d3502ae`

## Findings (honest)

- Grounded structured QA ingest works on the tested corpus shape: the batch row writes Oracle Base metadata and one Recurrence CF occurrence, then `calyx_oracle::reverse_query` returns the expected grounded cause.
- Edge checks passed: malformed `oracle.domain` and negative `oracle.t_secs` fail with `CALYX_CLI_USAGE_ERROR`; reingesting the same structured row does not duplicate recurrence occurrences.
- A storage bug was found and fixed while proving the path: durable SST checkpoint writes now collapse duplicate same-CF/same-key pending rows to the latest row, matching memtable/MVCC semantics.
- This does not retrofit the already-running #869 anchored ingest. That run uses the pre-#885 release binary and rows without the new `oracle` JSONL object; a later structured pass is required if the full anchored corpus needs Oracle recurrence rows.

## Conclusion & next step

#885 acceptance is satisfied for a structured corpus: persisted Base metadata plus Recurrence CF rows produce a grounded `reverse_query` cause. The next biomedical discovery issues can build corpus-wide structuring or ranked hypothesis workflows on top of this ingest path after #869 finishes.
