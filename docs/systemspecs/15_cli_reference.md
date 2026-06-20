# 15. CLI Reference (calyx-cli / `calyx`)

This document is an exhaustive, source-traced reference for the `calyx`
command-line binary produced by the `calyx-cli` crate. Every command, flag, and
side effect documented here was read from the crate source. Where the source
does not determine a value, the entry reads "Not determined from source".

> Sibling documents: storage internals in
> [05_aster_storage.md](05_aster_storage.md), the ledger/provenance model in
> [11_ledger_provenance.md](11_ledger_provenance.md), lenses/registry in
> [07_registry_lenses.md](07_registry_lenses.md), search in
> [08_sextant_search.md](08_sextant_search.md). The MCP server surface (a
> sibling front-end over the same engine) is documented in
> `16_mcp_server.md` (see [16_mcp_server.md](16_mcp_server.md)) — not produced
> by this crate.

## Source files covered

The crate has 186 files / ~39K LOC. This reference documents every top-level
subcommand and its direct flags. The command-defining sources are:

- `crates/calyx-cli/src/main.rs` — module list, `fn main`.
- `crates/calyx-cli/src/entry.rs` — process entry; intercept ordering.
- `crates/calyx-cli/src/dispatch.rs` — **the central manual arg dispatcher**
  (positional pattern match over `args.as_slice()`).
- `crates/calyx-cli/src/usage.rs` — the canonical `usage()` help string.
- `crates/calyx-cli/src/verify_restore.rs`,
  `crates/calyx-cli/src/healthcheck_daemon.rs` — pre-dispatch interceptors
  that own their own exit codes.
- `crates/calyx-cli/src/healthcheck.rs` — deploy-health command.
- `crates/calyx-cli/src/migrate/mod.rs` — `migrate` group.
- `crates/calyx-cli/src/leapable/mod.rs` (+ `read_flip.rs`, `dual_write.rs`,
  `issue612_fsv.rs`, `recall_comparator.rs`, `round_trip_verifier*`,
  `shadow_removal*`, `shadow_harness_cli.rs`) — `leapable` group.
- `crates/calyx-cli/src/anneal_commands.rs` (+ `anneal_*.rs`) — `anneal` group.
- `crates/calyx-cli/src/navigate/mod.rs` — `navigate` group.
- `crates/calyx-cli/src/sextant_commands.rs`,
  `media_commands.rs`, `lodestar_commands.rs`, `lens_commands.rs`,
  `panel_commands.rs`, `summarize_command.rs`,
  `intelligence_commands.rs` — domain command groups.
- `crates/calyx-cli/src/oracle_readback.rs`, `ph42_readback.rs`,
  `temporal_log_recurrence_readback/`, and the many `*_readback.rs` modules —
  `readback` topics.
- `crates/calyx-cli/src/{ops,fsv,crash,merkle,verify,scan,provenance,resource_status,resource_drill}.rs`
  — storage/FSV/crash tooling reachable directly from `dispatch.rs`.

---

## 1. Invocation, parsing model, and global behavior

**Binary name:** `calyx` (from `crates/calyx-cli`, `fn main` in
`main.rs` delegating to `entry::main`).

**Invocation:** `calyx <command> [subcommand] [flags...]`

**Parser model.** There is **no clap**. `dispatch::run` (`dispatch.rs`)
matches the raw `Vec<String>` of arguments against exact positional patterns
with literal-string guards (e.g. a pattern matches only when
`flag == "--vault"` and the flags appear *in the listed order*). Consequences:

- Flag **ordering is significant** for the top-level `dispatch.rs` matches.
  Several module sub-dispatchers (`navigate`, `lens`, `healthcheck`, `migrate`,
  `anneal_*`) use their own order-independent `--key value` parsers.
- Unknown / mis-ordered invocations fall through to
  `Err(CliError::usage(usage::usage()))`, printing the full usage string.

**Entry interception order** (`entry.rs`):

1. `verify_restore::try_run` — intercepts `verify-restore` (owns exit codes).
2. `healthcheck_daemon::try_run` — intercepts `healthcheck --config …` only
   (owns 0/1/2 exit codes).
3. Otherwise `dispatch::run`; `Ok(())` → `ExitCode::SUCCESS`, `Err(e)` →
   `e.emit()` (`error.rs`).

**Global flags / help.** There are no true global flags. With no arguments, or
with `-h` / `--help` as the sole argument, `usage::print_usage()` prints the
usage text and exits success (`dispatch.rs` lines 489–496).

**Global environment variables** (read across commands):

| Env var | Used by | Effect |
|---|---|---|
| `CALYX_HOME` | `lens`, `panel` | Root for lens catalog `lenses/registry.json`. |
| `CALYX_LEDGER_DIR` | `merkle-root --range` | Ledger dir when no `--ledger`/`--vault`. |
| `CALYX_HEALTH_LOG_PATH`, `CALYX_SECRET_ENV`, `CALYX_HEALTH_VAULT`, `CALYX_HEALTH_METRICS_URL` | `healthcheck` | Defaults for health probe paths/URL. |
| `CALYX_PANEL_VRAM_SOFT_CAP_BYTES`, `CALYX_TEI_RESERVED_BYTES`, `CALYX_PANEL_RAM_SOFT_CAP_BYTES`, `CALYX_CPU_LENS_POOL_CAP` | `lens add` | Placement budget caps. |

**Output convention.** Most commands print a JSON report to stdout
(`output::print_json`). FSV/readback commands additionally write a
Source-of-Truth file and re-read + BLAKE3-digest it before printing.

### Top-level command list (from `dispatch.rs` + interceptors)

`readback`, `healthcheck`, `migrate`, `intelligence`, `leapable`, `navigate`,
`sextant`, `media`, `lodestar`, `lens`, `panel`, `summarize`, `anneal`, `ward`,
`merkle-root`, `verify-chain`, `verify-restore`, `scan`, `ledger-tail`,
`get-provenance`, `get-answer-trace`, `audit`, `compact`, `compact-watch`,
`soak`, `tier`, `resource-status`, `resource-drill`, `vault-demo`, `arrow-demo`,
`cf-demo`, `mvcc-demo`, `wal-drill`, `wal-replay`, `crash-drill`, `recover`,
`open-check`, `corrupt-shard`, `wal-batch-demo`.

---

## 2. `readback` — Source-of-Truth inspection (FSV)

`readback` is the largest command surface: it prints source-of-truth bytes /
JSON for manual Functional Source Verification. It is **read-only** unless a
topic explicitly persists fixtures (oracle topics; see §2.4). Dispatched in
`dispatch.rs` and delegated to many `*_readback.rs` modules.

### 2.1 Byte / structural readback (`dispatch.rs`)

| Form | Flags / args | Module · fn | Side effects |
|---|---|---|---|
| `readback --hex <file>` | `<file>` path | `cli_support::readback_hex` | Reads file, prints hex. |
| `readback --vault-tree <dir>` | `<dir>` | `vault_tree::readback_vault_tree` | Reads vault dir tree. |
| `readback --vault <dir> --verify-against <sqlite.db>` | paths | `leapable::readback_dual_write_verify` | Reads vault + SQLite; prints verify JSON. |
| `readback --vault <dir> --show-manifest` | path | `leapable::readback_shadow_manifest` | Reads MANIFEST. |
| `readback vault-manifest --field <name> --vault <dir>` | `--field`, `--vault` | `manifest_readback::readback_vault_manifest_field` | Reads one manifest field. |
| `readback --cf <name> --vault <dir>` | `--cf`, `--vault` | `ops::readback_cf` | Reads a column family. |
| `readback --cf ledger --vault <dir> --seq <n>` | `--seq` (u64) | `verify::readback_ledger_seq` | Reads ledger entry at seq. |
| `readback --cf <name> --level <dir>` | `--cf`, `--level` | `fsv::readback_level` | Reads an SST level dir. |
| `readback --wal --vault <dir>` | `--wal`, `--vault` | `ops::readback_wal` | Reads WAL. |
| `readback config <tripwire\|budget> --vault <dir>` | name, `--vault` | `cli_support::readback_config` | Reads config. |
| `readback ledger --kind Anneal --action <...> --last <n> --vault <dir>` | (passed through) | `anneal_ledger_readback::run` | Reads anneal ledger. |

### 2.2 Time-travel / temporal / dedup / recurrence topics

| Form | Required flags | Module · fn |
|---|---|---|
| `readback temporal_search --explain --clock-fixed <secs> --tz-offset <secs>` | both ints | `temporal_readback::readback_temporal_search` |
| `readback dedup-check --vault <dir> --cx-id <cx> --slot <n> --tau <f> --near-cos <f> --distinct-cos <f> --vault-id <id> --salt <s>` | all 8 | `dedup_readback::readback_dedup_check` |
| `readback dedup-audit --vault <dir> --cx-id <cx>` | both | `dedup_audit_readback::readback_dedup_audit` |
| `readback dedup-undo --vault <dir> --token <json>` | both | `dedup_audit_readback::readback_dedup_undo` |
| `readback cx-list --vault <dir>` | `--vault` | `dedup_audit_readback::readback_cx_list` |
| `readback kernel-health --root <dir> --kernel-id <cx>` | both | `kernel_health_readback::readback_kernel_health` |
| `readback recurrence-series --vault <dir> --cx-id <cx>` | both | `recurrence_readback::readback_recurrence_series` |
| `readback periodic-recall --vault <dir> (--hour <0-23> \| --day <0-6>) [...]` | variadic | `recurrence_readback::readback_periodic_recall` |
| `readback time-prediction --vault <dir> --cx-id <cx> --confidence-ceiling <f>` | all 3 | `time_prediction_readback::readback_time_prediction` |
| `readback time-index --vault <dir>` | `--vault` | `timetravel_readback::readback_time_index` |
| `readback as-of --vault <dir> --t-millis <ms>` | both | `timetravel_readback::readback_as_of` |
| `readback trigger-audit <sub_id> --vault <dir>` | positional + `--vault` | `trigger_readback::readback_trigger_audit` |
| `readback trigger-fired --vault <dir>` | `--vault` | `trigger_readback::readback_trigger_fired` |
| `readback anneal mistakes --vault <dir> --last <n>` | both | `anneal_mistakes_readback::readback_mistakes` |

All of the above are read-only against the named vault.

### 2.3 PH42 artifact readback topics (`ph42_readback.rs`)

Topics recognized by `ph42_readback::is_topic`: `assay-report`,
`temporal-cross-term`, `kernel-weights`, `kernel-window`, `ward-novelty`,
`compression-ratio`, `compression-report` (artifact kind `ph59...`),
`anneal-schedule`. Each maps to a versioned `artifact_kind`
(`ph42.<topic>.v1`).

`readback <topic> --artifact <json> [--field <path>]`

| Flag | Type | Required | Default | Description |
|---|---|---|---|---|
| `--artifact` | path | Yes | — | JSON artifact to read. |
| `--field` | dotted path | No | (whole doc) | Extract a nested field. |

Side effects: reads the artifact, validates `surface`/`artifact_kind`/
`schema_version`/`source_of_truth`, computes the file's BLAKE3, prints readback
JSON. Read-only.

### 2.4 Oracle readback topics (`oracle_readback.rs`)

Topics recognized by `oracle_readback::is_topic`: `oracle_self_consistency`,
`oracle_sufficiency`, `oracle_predict`, `oracle_expand`, `reverse_query`,
`super_intelligence`. **These topics persist fixture rows into the vault** (they
open a durable vault, write base/recurrence/assay rows, then `flush()`), so they
are *not* purely read-only.

Common flags (all topics): `--vault <dir>`, `--vault-id <id>`, `--salt <s>` (all
required). Topic-specific:

| Topic | Extra required flags | Optional | Module |
|---|---|---|---|
| `oracle_self_consistency` | `--domain <d>` | — | `oracle_readback.rs` |
| `oracle_sufficiency` | `--fixture <json>` | — | `oracle_readback.rs` |
| `oracle_predict` | `--fixture <json>` | — | `oracle_readback/predict.rs` |
| `oracle_expand` | `--fixture <json>` | `--depth <0-4>` (default MAX_DEPTH) | `oracle_readback/butterfly.rs` |
| `reverse_query` | `--domain <d> --answer <text> --fixture <json>` | — | `oracle_readback/reverse_query.rs` |
| `super_intelligence` | `--domain <d> --fixture <json>` | — | `oracle_readback/super_intelligence.rs` |

Side effects: open vault → persist fixture rows → call the corresponding
`calyx_oracle` routine → read back ledger rows → `vault.flush()` → print JSON.

### 2.5 `readback temporal-log-recurrence` (`temporal_log_recurrence_readback/`)

`readback temporal-log-recurrence --log <csv> --vault <dir> --out <json> --rows <n> --expected-cadence-secs <secs> --confidence-ceiling <f>`

| Flag | Type | Required | Notes |
|---|---|---|---|
| `--log` | path (CSV) | Yes | `YYYY-MM-DD HH:MM:SS,...` rows, strictly monotonic. |
| `--vault` | path | Yes | **Must be empty or nonexistent** (created fresh). |
| `--out` | path | Yes | Artifact JSON written here. |
| `--rows` | usize | Yes | Min 3. |
| `--expected-cadence-secs` | i64 | Yes | Must be positive. |
| `--confidence-ceiling` | f32 | Yes | In `[0,1]`. |

Side effects: creates a vault with fixed salt `issue610-temporal-real-log`,
ingests events with dedup, computes recurrence + next-occurrence prediction,
**writes the artifact JSON to `--out`**, prints it.

---

## 3. `migrate` — SQLite → Calyx vault migration (`migrate/mod.rs`)

Order-independent flag parser. `print_json` output. Subcommands:

### `migrate vault <sqlite.db> <vault.calyx> [flags]`

| Flag | Type | Req | Default | Description |
|---|---|---|---|---|
| `<sqlite.db>` | path (pos 0) | Yes | — | Source SQLite. |
| `<vault.calyx>` | path (pos 1) | Yes | — | Destination vault. |
| `--verify` | bool | No | false | Verify byte-exact content after write. |
| `--dry-run` | bool | No | false | Plan only; no vault writes. |
| `--backfill-default-panel` | bool | No | false | Backfill default panel embeddings. |
| `--offline-backfill` | bool | No | false (RealTei) | Use `OfflineDeterministic` backfill mode instead of TEI. |
| `--gte-lens-id <hex16>` | LensId | No | adapter default | Base lens id; must match existing manifest. |
| `--gte-endpoint <url>` | string | No | — | TEI endpoint (logged). |
| `--batch-size <n>` | usize | No | **100** | Rows per batch (min 1). |

Side effects: opens/streams SQLite, writes constellations into the durable
Aster vault, writes/updates the migration manifest, `flush()`es, optionally
backfills (may call TEI when not offline) and verifies; emits `eprintln!`
progress. Fails closed on content-hash mismatch.

### `migrate backfill <sqlite.db> <vault.calyx> [--offline-backfill] [--batch-size <n>]`

Defaults: `backfill=true`, `--batch-size` default **16**. Backfills the default
panel for an already-migrated vault; writes embeddings, returns `BackfillSummary`.

### `migrate verify <sqlite.db> <vault.calyx> [--require-backfill]`

Read-only verification; `--require-backfill` (bool) demands backfilled rows.
Prints per-row `MISMATCH` lines and a failing error on any mismatch.

### `migrate status <vault.calyx>`

One positional arg. Reads manifest + vault, prints `StatusReport`.

### `migrate readback <sqlite.db> <vault.calyx> <chunk_id>`

Three positional args. Reads one chunk from SQLite and the matching vault row,
prints JSON. Read-only.

---

## 4. `leapable` — PostgreSQL/SQLite shadow-cutover tooling (`leapable/`)

Dispatched by `leapable::run`. All print JSON; most write evidence files. See
[11_ledger_provenance.md](11_ledger_provenance.md) for the ledger semantics.

### `leapable dual-write --sqlite <db> --calyx <dir> [--inject-shadow-failure]`

| Flag | Type | Req | Default |
|---|---|---|---|
| `--sqlite` | path | Yes | — |
| `--calyx` | path | Yes | — |
| `--inject-shadow-failure` | bool | No | false |

Side effects: initializes/migrates the Aster vault under `<calyx>/aster/`,
writes `<calyx>/DUAL_WRITE_RECEIPTS.jsonl`, updates MANIFEST, flushes, prints
`DualWriteReport`.

### `leapable read-flip --sqlite <db> --calyx <dir> [--tau <f>] [--skip-backfill]`

| Flag | Type | Req | Default |
|---|---|---|---|
| `--sqlite` | path | Yes | — |
| `--calyx` | path | Yes | — |
| `--tau` | f32 | No | **0.72** |
| `--skip-backfill` | bool | No | false |

Side effects: flips MANIFEST read mode to Calyx, appends a ledger entry, prints
flip report. (`read_flip.rs::run_read_flip`, parser `parse_flip_args`.)

### `leapable ask --vault <dir> (--query-vector <json-array> | --query <text>) [--top-k <n>]`

| Flag | Type | Req | Default |
|---|---|---|---|
| `--vault` | path | Yes | — |
| `--query-vector` | JSON `[f32]` | one-of | — |
| `--query` | text | one-of | — |
| `--top-k` | usize | No | **5** |

Read-only; prints `AskResult` (ranked hits with provenance). (`read_flip.rs::run_ask`.)

### `leapable recall-compare --sqlite <db> --calyx <dir> --queries <jsonl> [--top-k <n>]`

| Flag | Type | Req | Default |
|---|---|---|---|
| `--sqlite` | path | Yes | — |
| `--calyx` | path | Yes | — |
| `--queries` | path (JSONL) | Yes | — |
| `--top-k` | usize | No | **10** |

Side effects: writes `<calyx>/PARITY_REPORT.jsonl`; prints `ParityReport`.

### `leapable verify-round-trip --sqlite <db> --calyx <dir> [--output <json>] [--benchmark --queries <jsonl>] [--top-k <n>]`

| Flag | Type | Req | Default |
|---|---|---|---|
| `--sqlite` | path | Yes | — |
| `--calyx` | path | Yes | — |
| `--output` | path | No | — (stdout only) |
| `--benchmark` | bool | No | false |
| `--queries` | path | If `--benchmark` | — |
| `--top-k` | usize | No | **10** |

Read-only vs vault; writes JSON to `--output` if given. (`round_trip_verifier`.)

### `leapable remove-shadow --sqlite <db> (--calyx|--vault) <dir> --vault-type <text|code|civic|media>`

| Flag | Type | Req | Default |
|---|---|---|---|
| `--sqlite` | path | Yes | — |
| `--calyx` / `--vault` | path | Yes (either) | — |
| `--vault-type` | enum | Yes | — |

Side effects: updates MANIFEST, appends ledger entries, writes/reads
`<calyx>/aster/migration-panel.json`, archives SQLite to `<sqlite>.archive`
when in shadow mode. Prints `RemoveShadowReport`. (`shadow_removal/cli.rs`.)

### `leapable issue612-fsv --baseline-latency <json> --flipped-latency <json> --pg-before <dir> --pg-after <dir> --out <json>`

All five required. Side effects: creates parent dirs for `--out`, writes the
evidence artifact, re-reads + BLAKE3-digests it, prints readback. (`issue612_fsv.rs`.)

### `leapable shadow-open --sqlite <db> --vault <dir>` / `leapable shadow-readback --vault <dir>`

`shadow-open` requires `--sqlite` + `--vault`; `shadow-readback` requires only
`--vault`. Both read-only; print MANIFEST/contract JSON. (`shadow_harness_cli.rs`.)

---

## 5. `anneal` — annealing/intelligence diagnostics (`anneal_commands.rs`)

Dispatched by `anneal_commands::run`. Two readback forms use strict positional
order; the rest delegate to per-command parsers. All print JSON.

| Subcommand | Required | Optional (default) | Module | Writes? |
|---|---|---|---|---|
| `status --health --vault <dir>` | both | — | `anneal_status::status_health` | read-only |
| `status --faults --last <n> --vault <dir>` | all | — | `anneal_status::status_faults` | read-only |
| `replay-status --vault <dir>` | `--vault` | — | `anneal_replay_readback` | read-only |
| `head-status --kind <Predictor\|Calibrator\|FusionWeights> --vault <dir>` | both (any order) | — | `anneal_head_readback` | read-only |
| `bandit-status --key <shape_key> --vault <dir>` | both (any order) | — | `anneal_bandit_readback` | read-only |
| `ab-log --vault <dir> [--last <n>]` | `--vault` | `--last` (5) | `anneal_ab_log` | read-only |
| `soak --vault <dir> --corpus-jsonl <jsonl> --metrics-dir <dir> [--queries <n>] [--sample-interval <n>] [--min-docs <n>] [--vault-id <id>] [--salt <s>]` | 3 paths | `--queries` (1,000,000), `--sample-interval` (10,000), `--min-docs` (50,000), `--vault-id` (default ULID), `--salt` (`calyx-ph70-anneal-soak`) | `anneal_soak` | **writes metrics dir** |
| `soak-report --vault <dir> [--last <n>]` | `--vault` | `--last` (1) | `anneal_soak_report` | read-only |
| `autotune-report --scope <forge\|index\|storage> --cache <json> --vault <dir> [--last <n>] [--slot <n>]` | `--scope --cache --vault` | `--last` (5); `--slot` required for index scope, forbidden for storage | `anneal_autotune_report` | read-only |
| `intelligence-report --fixture <json> [--vault <dir>]` | `--fixture` | `--vault` (persists snapshots under `<vault>/.anneal/`) | `anneal_intelligence_report` | conditional write |
| `growth-curve --vault <dir> [--last <n>]` | `--vault` | `--last` (20) | `anneal_growth_curve` | read-only |
| `goodhart-check --fixture <json> --vault <dir> --vault-id <id> --salt <s>` | all 4 | — | `anneal_goodhart_check` | **writes goodhart state + ledger** |
| `deficit-map --anchor <id> --fixture <json> [--threshold <bits>]` | `--anchor --fixture` | `--threshold` (default constant) | `anneal_deficit_map` | read-only |
| `propose-preview --anchor <id> --deficit <json> --corpus <json>` | all 3 | — | `anneal_propose_preview` | read-only |
| `lens-proposal-log (--fixture <jsonl> \| --vault <dir>) [--last <n>]` | one-of | `--last` (5) | `anneal_lens_proposal_log` | read-only |
| `propose-lens-run --fixture <json>` | `--fixture` | — | `anneal_propose_lens_run` | per-fixture |
| `frozen-guard-report --artifact <json>` | `--artifact` | — | `anneal_frozen_guard_readback` | read-only |
| `regression-report --artifact <json>` | `--artifact` | — | `anneal_regression_readback` | read-only |

> Note: the usage string spells `autotune-report --scope` values as
> `forge | index | storage`; the parser (`anneal_autotune_report.rs::ReportRequest::parse`)
> enforces the scope/`--slot` coupling. Treat the usage string as the
> source-of-truth surface and the parser for exact accepted tokens.

---

## 6. `navigate` — engine navigation primitives (`navigate/mod.rs`)

Order-independent `--key value` parser (`Flags::parse`). Every mode reads a
deterministic engine spec from `--spec <json>`, rebuilds a `SearchEngine`, runs
the primitive, and prints readback JSON. With `--out <file>` it writes the JSON
to a Source-of-Truth file, re-reads it, and prints `NAVIGATE_OUT=` and
`NAVIGATE_BLAKE3=` lines. See [08_sextant_search.md](08_sextant_search.md).

Common to all modes: `--spec <json>` (required), `--out <json>` (optional, SoT
file write).

| Mode | Required flags | Optional |
|---|---|---|
| `neighbors` | `--cx <cx> --slot <n> --k <n>` | — |
| `define` | `--cx <cx> --slot <n> --k <n>` | — |
| `agree` | `--anchor <cx> --k <n>` | `--slots <a,b>` |
| `disagree` | `--anchor <cx> --k <n>` | `--slots <a,b>` |
| `traverse` | `--anchor <cx> --direction <forward\|backward\|both> --hops <1-10>` | — |
| `skills` | — | `--min-cluster-size <n>` (2), `--min-samples <n>` (1), `--max-constellations <n>`, `--slots <a,b>`, `--allow-single` |
| `search-skill` | `--skill <name> --slot <n> --k <n> --vec <a,b>` | `--text <s>` (default `navigate-search-skill`), `--min-cluster-size`, `--min-samples` |

Side effects: read-only on the spec; only `--out` causes a file write.

---

## 7. `sextant` / `media` / `lodestar` — validation harnesses

All emit JSON evidence and write metrics directories (FSV harnesses). They open
durable vaults with default `--vault-id`/`--salt` unless overridden.

### `sextant recall-validate` (`sextant_recall_validation`)

`--corpus-jsonl --queries-jsonl --qrels --metrics-dir --vault` (all required);
optional `--query-limit` (50), `--k` (10), `--min-delta` (0.15),
`--vault-id` (default ULID), `--salt` (`calyx-ph70-sextant-recall`). Creates the
metrics dir; writes metric outputs; prints evidence.

### `sextant diskann-validate` (`sextant_diskann_validation`)

`--root` (required); optional `--mode` (`happy`; also
`empty|dim-mismatch|truncated|missing-raw`), `--nodes` (1000), `--dim` (64),
`--queries` (128), `--k` (10), `--beamwidth` (32), `--ef-search` (128),
`--rescore-k` (64). Builds a DiskANN index under `<root>/idx/...` and writes
`<root>/metrics/diskann_*.json`.

### `media image-validate` (`media_image_validation`)

`--samples --metrics-dir --vault` (required); optional `--min-image-bits`
(0.05), `--min-cross-modal-bits` (0.05), `--k` (3), `--vault-id`,
`--salt` (`calyx-ph70-media-image`). Creates metrics dir; writes outputs.

### `media emotion-validate` (`media_emotion_validation`)

`--samples --metrics-dir --vault` (required); optional `--min-bits` (0.05),
`--k` (3), `--vault-id`, `--salt` (`calyx-ph70-media-emotion`).

### `media video-validate` / `media video-readback` (`media_video_validation`)

`video-validate`: `--metadata --metrics-dir --vault` (required); optional
`--dataset-root`, `--vault-id`, `--salt` (`calyx-ph70-media-video`). Reads
real video files, validates extension/signature/`ffprobe` metadata, retains
source bytes, writes video rows/slot vectors/Online evidence, and prints JSON.

`video-readback`: `--vault` (required); optional `--vault-id`, `--salt`. Scans
Base rows, verifies retained blob SHA256/byte counts, and decodes video slot
rows for manual FSV.

### `lodestar kernel-validate` (`lodestar_commands`)

`--corpora-dir --metrics-dir` (required); optional `--query-limit` (500),
`--top-k` (10), `--min-ratio` (0.95). Reads corpora, writes metrics, prints JSON.

---

## 8. `lens` / `panel` — registry/panel inspection

See [07_registry_lenses.md](07_registry_lenses.md). Order-independent parsers.
Catalog path resolves to `<CALYX_HOME|--home>/lenses/registry.json`.

### `lens add --manifest <manifest.json> [--home <dir>]`

Reads lens spec, estimates cost, chooses placement (uses the
`CALYX_PANEL_*`/`CALYX_TEI_RESERVED_BYTES`/`CALYX_CPU_LENS_POOL_CAP` env caps),
**writes/rewrites the catalog JSON**, prints `AddReport`. Rejects `--input`/`--repeat`.

### `lens list [--home <dir>]`

Read-only; prints `ListReport`. Rejects `--manifest`, `--input`, `--repeat`.

### `lens commission --hf <id> --runtime <onnx-int8|onnx-fp32|candle-fp16|tei> [flags]`

First-class LensForge commissioning pipeline. Writes artifacts under `--out`
or `<CALYX_HOME|--home>/lenses/commissioned/<hf>-<runtime>`, writes
`conversion-log.jsonl`, writes `lensforge.manifest.json`, then registers it via
the same hash-verified catalog path as `lens add`. Optional flags: `--name`,
`--endpoint` (TEI), `--dim`, `--license`, `--non-commercial`, `--pooling`,
`--norm`, `--quant-target`, `--max-batch`. `--max-batch` writes the manifest
batch ceiling that later scale runs clamp to after it is proven. `onnx-int8`
runs Optimum export plus ONNX Runtime quantization. `onnx-fp32` runs the same
Optimum feature-extraction export without the quantization step and writes an
`onnx`/`f32` manifest for strict batch-stable GPU lenses when dynamic int8
quantization drifts.

### `lens explain --manifest <manifest.json> [--input <text>|--input-file <path>] [--repeat <n>] [--full-vector]`

Not in the top-level usage string but present in `lens_commands.rs::run`.
Supports `static_lookup`, `tei`, `candle-fp16`, `onnx-int8`, `onnx-fp32`, and
`multimodal-adapter` LensForge manifests. `--repeat` default 1 (must be > 0);
`--input` default `Calyx lens explain probe`; `--input-file` reads binary bytes
for media lenses and is mutually exclusive with `--input`. Read-only;
instantiates the real runtime, measures the probe, validates finite/dim/norm
against the frozen spec, and prints
`ExplainReport` (timing/norm/first values/runtime detail). `--full-vector`
adds the complete dense vector to the report after validation for FSV cosine
readback; it fails closed for non-dense outputs. `--home` not used by `explain`.

### `panel status [--home <dir>] | [--vault <dir>]`

`--home` and `--vault` are mutually exclusive. With `--vault`, loads the vault
panel state and prints `VaultPanelStatusReport`; otherwise reads the catalog and
prints `PanelStatusReport` (per-lens RAM/VRAM/placement). Read-only.

---

## 9. `summarize` — kernel summarization (`summarize_command.rs`)

`summarize --vault <dir> --scope <json|@file> --out <json> [flags]`

| Flag | Type | Req | Default | Description |
|---|---|---|---|---|
| `--vault` | path | Yes | — | Vault dir. |
| `--scope` | JSON or `@file` | Yes | — | Scope spec inline or from file. |
| `--out` | path | Yes | — | Output JSON (parent dirs created). |
| `--graph` | string | No | `DEFAULT_ASTER_ASSOC_COLLECTION` | Association collection. |
| `--as-of` | u64 | No | latest | Snapshot timestamp (ms). |
| `--anchor-label` | string | No | — | Anchor label. |
| `--max-kernel-size` | usize | No | — | Kernel size cap. |
| `--require-grounded` | bool | No | false | Require grounded result. |
| `--cache-ttl-secs` | u64 | No | — | Cache TTL. |
| `--recall-top-k` | usize | No | — | Recall benchmark top-k. |
| `--recall-held-out` | f32 | No | — | Held-out fraction. |
| `--recall-seed` | u64 | No | — | RNG seed. |
| `--recall-min-ratio` | f32 | No | — | Min recall ratio. |
| `--vault-id` | VaultId | No | default ULID | Vault id. |
| `--salt` | bytes | No | `calyx-summarize-cli` | Salt. |

Side effects: opens the vault, summarizes, `flush()`es, **writes JSON to
`--out`**, prints result. See [10_graph_kernel.md](10_graph_kernel.md).

`intelligence abundance --vault <dir>` (`intelligence_commands.rs`): reads
`<vault>/intelligence/abundance.json` and prints it. Read-only.

---

## 10. Ledger / Merkle / provenance commands

See [11_ledger_provenance.md](11_ledger_provenance.md). All dispatched directly
in `dispatch.rs`.

| Command | Flags | Module · fn | Effect |
|---|---|---|---|
| `merkle-root --ledger <dir> --range <a..b>` | `--ledger`, `--range` | `merkle::print_root` | Print Merkle root over ledger dir. Read-only. |
| `merkle-root --vault <dir> --range <a..b>` | `--vault`, `--range` | `merkle::print_root_from_vault` | Reads vault cf/ledger + WAL; **no side ledger dir created**. |
| `merkle-root --range <a..b>` | `--range` (+ `CALYX_LEDGER_DIR` env) | `merkle::print_root_from_env` | Ledger dir from env. |
| `verify-chain --ledger <dir> --range <a..b>` | `--ledger`, `--range` | `verify::verify_ledger_dir` | Verify hash chain in dir. |
| `verify-chain --vault <dir> --range <a..b>` | `--vault`, `--range` | `verify::verify_vault` | Verify vault chain. |
| `scan --cf ledger --vault <dir>` | fixed cf, `--vault` | `scan::scan_ledger_vault` | Scan ledger CF. |
| `ledger-tail --vault <dir> --last <n>` | both (`<n>` usize) | `scan::tail_ledger_vault` | Tail last n ledger entries. |
| `get-provenance --vault <dir> --cx <cx-id>` | both | `provenance::get_provenance` | Print provenance for cx. |
| `get-answer-trace --vault <dir> --answer <id-or-hex>` | both | `provenance::get_answer_trace` | Print answer trace. |
| `audit --vault <dir> --kind <kind>` | both | `provenance::audit` | Run audit of kind. |

`--range` uses `a..b` syntax (`merkle::parse_range` / `verify::parse_verify_range`).

---

## 11. `verify-restore` (interceptor; `verify_restore.rs`)

`verify-restore --vault <path> [--json]`

Intercepted in `entry.rs` **before** the generic dispatcher and owns its exit
codes: `0` = chain intact AND constellations/anchors/WAL bytes present; `1` =
any verification or usage failure (exact `CALYX_*` code on stderr). `--json`
emits the report as pretty JSON; otherwise a `VERIFY_RESTORE ...` text block.
Read-only. Implemented via `calyxd::verify::verify_restore`.

---

## 12. `healthcheck` — two distinct modes

`healthcheck` has two behaviors selected by presence of `--config`.

### 12.1 Daemon readiness: `healthcheck --config <calyx.toml> [--wait <secs>] [--out <json>]`

Intercepted by `healthcheck_daemon::try_run` (only when `--config` is present).
Owns exit codes: `0` healthy · `1` ran-but-unhealthy (JSON records the `CALYX_*`
code) · `2` could-not-run (bad args/config or SoT unwritable).

| Flag | Type | Req | Default |
|---|---|---|---|
| `--config` | path | Yes | — |
| `--wait` | u32 secs | No | config `healthcheck_timeout_secs` |
| `--out` | path | No | config `health_log_path` |

Side effects: real CUDA init + NVML VRAM check + vault read, then writes the
`CalyxHealthResult` JSON to the resolved out path (`calyxd::health`).

### 12.2 Deploy health: `healthcheck [flags]` (no `--config`) — `healthcheck.rs`

| Flag | Type | Req | Default |
|---|---|---|---|
| `--wait <secs>` | u64 | No | 0 |
| `--out <json>` | path | No | `CALYX_HEALTH_LOG_PATH` or `/zfs/hot/logs/calyx-health/latest.json` |
| `--secret-env <env>` | path | No | `CALYX_SECRET_ENV` or `/run/leapable/secrets/calyx.env` |
| `--calyx-home <dir>` | path | No | `CALYX_HOME` or `/home/croyse/calyx` |
| `--vault <dir>` | path | No | `CALYX_HEALTH_VAULT` (else skipped) |
| `--metrics-url <url>` | http URL | No | `CALYX_HEALTH_METRICS_URL` (else skipped) |
| `--require-env <name>` | string (repeatable) | No | `HF_HUB_TOKEN`, `HF_TOKEN` |

Side effects: probes calyx-home, secret-env (mode `0400` on Unix + required env
names), optional vault restore-readback, optional metrics scrape over **raw TCP
HTTP** (checks `calyx_ledger_chain_verify_ok == 1`). **Writes `latest.json` and
reads it back**; loops once per second until pass or `--wait` elapses. Failure
returns `CALYX_HEALTHCHECK_FAILED`.

---

## 13. Storage / FSV / crash / drill tooling (`dispatch.rs` → `ops`/`fsv`/`crash`)

All take strict positional flag order. Most mutate or stress the named vault;
treat as operator/FSV tooling. See [05_aster_storage.md](05_aster_storage.md).

| Command | Flags | Module · fn | Effect |
|---|---|---|---|
| `compact --vault <dir> --cf <name>` | both | `ops::compact` | Compacts a CF. |
| `compact-watch --vault <dir> --duration <30s\|500ms>` | both | `ops::compact_watch` | Watches compaction for a duration (`ops::parse_duration`). |
| `soak --vault <dir> --ops <n> --threads <n>` | all (usize) | `ops::soak` | Multi-threaded write soak. |
| `tier --vault <dir> --cf <name> --output <hot\|cold>` | all | `ops::tier` | Tier a CF hot/cold. |
| `vault-demo --vault <dir>` | `--vault` | `ops::vault_demo` | Demo writes. |
| `wal-batch-demo --vault <dir> --requests <n>` | both | `ops::wal_batch_demo` | WAL batch demo. |
| `arrow-demo --vault <dir>` | `--vault` | `fsv::arrow_demo` | Arrow demo. |
| `cf-demo --vault <dir>` | `--vault` | `fsv::cf_demo` | CF demo. |
| `mvcc-demo --vault <dir>` | `--vault` | `fsv::mvcc_demo` | MVCC demo. |
| `wal-drill --vault <dir> --records <n>` | both | `fsv::wal_drill` | WAL drill. |
| `wal-replay <wal-dir>` | positional | `fsv::wal_replay` | Replay a WAL dir. |
| `corrupt-shard --vault <dir> --cf <name> --byte-offset <n>` | all (u64) | `fsv::corrupt_shard` | **Deliberately corrupts** a shard byte. |
| `crash-drill --vault <dir> --point <P> [--pause-ms <n>]` | `--point` enum | `crash::crash_drill` | Simulated crash at point. |
| `recover --vault <dir>` | `--vault` | `crash::recover` | Recover after crash. |
| `open-check --vault <dir> --index <n>` | `--index` u8 | `crash::open_check` | Open-check at index. |

`crash-drill --point` values (`crash::CrashPoint::parse`): `before-wal-fsync`,
`after-wal-before-commit`, `after-commit-before-manifest`.

### Resource commands

| Command | Flags | Module · fn |
|---|---|---|
| `resource-status --vault <dir> [--metrics]` | `--vault`; `--metrics` selects Prometheus-style output (default JSON) | `resource_status::run_resource_status` |
| `resource-drill --vault <dir> --ops <n> --value-bytes <n> --memtable-cap <bytes> --pin-max-age-ms <ms>` | all required | `resource_drill::run_resource_drill` |

### `ward` readback

`ward tau --slot <n> --vault <dir>` (`ward_tau_readback::readback_ward_tau`) —
reads the tau threshold for a slot. Read-only. See
[12_ward_guard.md](12_ward_guard.md).

---

## 14. Exit codes and errors

- Generic path: `Ok(())` → `ExitCode::SUCCESS`; any `CliError` →
  `error::CliError::emit()` (prints `CALYX_*` code + remediation to stderr).
- `verify-restore`: `0` pass / `1` fail (own contract).
- `healthcheck --config`: `0` healthy / `1` unhealthy / `2` cannot-run.
- `healthcheck` (deploy): non-zero on `CALYX_HEALTHCHECK_FAILED`.
- Exact non-zero numeric codes for the generic dispatcher path beyond
  success/failure are **Not determined from source** here (driven by
  `CliError::emit`, defined in `error.rs`).

## 15. Coverage statement

Every **top-level subcommand** and its **direct flags** are documented above,
traced to source file + function. For the large `readback` and `anneal` groups,
each topic/subcommand is listed with its required/optional flags and defaults.
The only sub-detail intentionally summarized rather than fully expanded is the
internal JSON *fixture schema* consumed by the oracle/anneal fixture-driven
topics (e.g. `oracle_predict --fixture`, `super_intelligence --fixture`); those
schemas live in the respective `oracle_readback/*` and `anneal_*` modules and
are described at field-group level in §2.4. No top-level command has been
omitted.
