# 20. CLI and Daemon Reference (`calyx-cli` + `calyxd`)

**Source files covered:**

calyx-cli (binary `calyx`):
- `crates/calyx-cli/src/main.rs`
- `crates/calyx-cli/src/entry.rs`
- `crates/calyx-cli/src/dispatch.rs`
- `crates/calyx-cli/src/usage.rs`
- `crates/calyx-cli/src/error.rs`
- `crates/calyx-cli/src/cmd/mod.rs`
- `crates/calyx-cli/src/cmd/vault.rs`, `cmd/ingest.rs`, `cmd/search.rs`, `cmd/intelligence.rs`, `cmd/provenance.rs`, `cmd/lens.rs`, `cmd/readback.rs`
- `crates/calyx-cli/src/verify_restore.rs`
- `crates/calyx-cli/src/healthcheck_daemon.rs`
- `crates/calyx-cli/src/healthcheck.rs`
- `crates/calyx-cli/src/anneal_commands.rs` (+ `anneal_*` handler modules)
- `crates/calyx-cli/src/navigate/`, `sextant_commands.rs`, `media_commands.rs`, `lodestar_commands.rs`, `lens_commands.rs`, `panel_commands.rs`, `intelligence_commands.rs`, `summarize_command.rs`, `migrate/`
- `crates/calyx-cli/src/leapable/`
- `crates/calyx-cli/src/oracle_readback.rs`, `ph42_readback.rs`, `crash.rs`, `ops.rs`, `fsv.rs`, `verify.rs`, `merkle.rs`, `scan.rs`, `provenance.rs`, `resource_status.rs`, `resource_drill.rs`

calyxd (binary `calyxd`):
- `crates/calyxd/src/lib.rs`, `main.rs`, `startup.rs`
- `crates/calyxd/src/config.rs`, `error.rs`
- `crates/calyxd/src/cuda_probe.rs`, `vram.rs`
- `crates/calyxd/src/health.rs`
- `crates/calyxd/src/metrics.rs`, `metrics/calyx.rs`, `metrics/hazards.rs`, `metrics/zfs.rs`
- `crates/calyxd/src/server.rs`, `mcp_server.rs`
- `crates/calyxd/src/verify.rs`, `verify_loop.rs`

Cross-references: error catalog roots in [03_configuration.md](03_configuration.md); subsystem entry points in [01_system_overview.md](01_system_overview.md); MCP dispatch in [19_mcp_api_tools_reference.md](19_mcp_api_tools_reference.md); ledger verify in [14_ledger_provenance.md](14_ledger_provenance.md); restore byte-layout in [04_storage_and_schema.md](04_storage_and_schema.md).

---

## 1. Entry points / binaries

| Crate | `[[bin]]` name | Path | Purpose |
|---|---|---|---|
| `calyx-cli` | `calyx` | `crates/calyx-cli/src/main.rs` | The operator/agent CLI (~229 `.rs` files). |
| `calyxd` | `calyxd` | `crates/calyxd/src/main.rs` | The daemon: chain-verify loop, `/metrics`, config-driven server mode. |

There are no console-script wrappers beyond these two Cargo `[[bin]]` targets. `calyxd` is also consumed as a **library** by `calyx-cli` (its `Cargo.toml` lists `calyxd = { path = "../calyxd" }`); the CLI reuses `calyxd::verify`, `calyxd::config`, and `calyxd::health` for `verify-restore` and the `healthcheck --config` form (`crates/calyxd/src/lib.rs`).

---

## Part A — `calyx-cli`

### A.1 Argument-parsing approach

`calyx-cli` uses **no `clap` / no derive parser** — all parsing is hand-rolled with `std::env::args()` and Rust slice-pattern matching. `Cargo.toml` declares no `clap` dependency. `fn main()` (`main.rs`) calls `entry::main()` (`entry.rs`), which dispatches through four layers in order, returning a `std::process::ExitCode`:

1. `verify_restore::try_run(&args)` — intercepts `verify-restore` (owns its 0/1 exit, ahead of the generic matcher).
2. `healthcheck_daemon::try_run(&args)` — intercepts `healthcheck --config <toml>` (the PH65 T04 daemon-readiness probe; owns a 0/1/2 exit contract).
3. `cmd::try_run(&args)` — the newer **PH62 structured subcommand** layer (`cmd/mod.rs`): `readback`, `healthcheck` (plain), and the 20 `is_cmd` commands. Returns `Option<CliResult>`.
4. `dispatch::run(args)` — the **legacy slice-pattern dispatcher** (`dispatch.rs`): `readback` topics, `merge`/`merkle-root`, FSV drills, ops, migrate, navigate, sextant, media, lodestar, lens, panel, anneal, summarize, leapable, ward.

`cmd/mod.rs::try_run` first tries `readback::try_run` and `healthcheck::try_run`, then checks `is_cmd(command)`; for the 20 structured commands it calls `parse(args).and_then(run)`. `verify-chain` with a leading `--` arg is deliberately handed to the legacy dispatcher (the range form lives there). Each structured command parses its own flags via a per-command loop with a shared `value(args, idx, flag)` helper.

#### Error / exit contract (`error.rs`)

Every CLI failure serializes to a stable JSON envelope on **stderr** and exits with code **2** (`CLI_ERROR_EXIT`). `CliError::emit()` writes `{"code":"…","message":"…","remediation":"…"}` (byte-identical field order to `CalyxError`'s serde).

| `CliError` variant | `code` | Remediation |
|---|---|---|
| `Calyx(CalyxError)` | the catalog `code` (PRD 18), carried verbatim | catalog `remediation` |
| `Io(String)` | `CALYX_CLI_IO_ERROR` | "check the path/permissions in the message and retry" |
| `Usage(String)` | `CALYX_CLI_USAGE_ERROR` | "run `calyx --help` and fix the command/flags shown in the message" |

`LodestarError` is mapped to `Calyx(...)` with the subsystem code preserved. `verify-restore`, `healthcheck --config`, and a few FSV tools bypass this envelope and print `error: …` to stderr with their own exit codes (noted per command).

`calyx`, `calyx --help`, or `calyx -h` print the full usage text (`usage::print_usage`) and exit 0. An unmatched command yields `CliError::usage(usage())` → exit 2.

### A.2 Structured subcommands (PH62 layer, `cmd/`)

These 20 commands are gated by `cmd::is_cmd`. Most resolve `<vault>` against `$CALYX_HOME/vaults/index.json` (by name), a `VaultId` (ULID), or a direct path. All output JSON to stdout unless noted.

#### A.2.1 `create-vault <name> [--panel-template <t>]` (`cmd/vault.rs`)
Creates a vault directory + index entry. Requires `CALYX_HOME`.

| Arg/flag | Type | Req | Default | Description |
|---|---|---|---|---|
| `<name>` (pos) | string | yes | — | Vault name; validated non-empty, no `/ \ . ..`, no whitespace. |
| `--panel-template` | enum | no | `text-default` | One of `text-default, code-default, civic-default, legal-default, medical-default, bio-default, media-default` (`PANEL_TEMPLATES`). |

Side effects: creates `$CALYX_HOME/vaults/{vault_id}`, updates `index.json`, initializes panel state. Prints `{vault_id, name, panel_template}`.

#### A.2.2 `add-lens <vault> --name <n> --runtime <r> [...]` (`cmd/vault.rs`)
| Arg/flag | Type | Req | Default | Description |
|---|---|---|---|---|
| `<vault>` (pos) | string | yes | — | Vault ref. |
| `--name` | string | yes | — | Lens name (path-safe). |
| `--runtime` | enum | yes | — | `algorithmic[:byte\|byte-features\|scalar\|ast-style\|one-hot:<n>]`, `tei-http`, `external-cmd`, `candle-local`, `onnx`, `multimodal-adapter`. |
| `--endpoint` | string/url | no | — | TEI URL or external-cmd path (or runtime-id). |
| `--weights` | path | no | — | Weights file. |
| `--shape` | string | no | runtime-dependent (`Dense(768)` TEI, `Dense(16)` external-cmd) | `Dense(<dim>)` or `Sparse(<dim>)`, dim > 0. |
| `--modality` | enum | no | `text` | `text, code, image, audio, video, structured, mixed` (also protein/dna/molecule accepted). |

Side effects: registers lens, persists panel state. Prints `{lens_id, slot_id, name}`.

#### A.2.3 `retire-lens <vault> --slot <u16>` / `park-lens <vault> --slot <u16>` (`cmd/vault.rs`)
Transition an Active slot to Retired/Parked. `--slot` (u16) required. Prints `{status, slot}`; persists panel state.

#### A.2.4 `list-panel <vault>` (`cmd/vault.rs`)
No flags. Prints a table: `slot, name, state, bits, ci_lo, ci_hi`.

#### A.2.5 `profile-lens [--name][--runtime][--endpoint][--weights][--shape][--modality][--probe]` (`cmd/vault.rs`)
Profiles a lens without persisting to any vault. Same flags as `add-lens` (all optional; defaults `--name profile-lens`, `--runtime algorithmic`, `--modality text`) plus `--probe <path>` (newline-delimited probe texts; default 3 built-in probes). Prints a lens profiling card (bits/cardinality).

#### A.2.6 `ingest <vault> (--text <s> | --batch <jsonl>) [--idempotent]` (`cmd/ingest.rs`)
| Flag | Type | Req | Default | Description |
|---|---|---|---|---|
| `<vault>` (pos) | string | yes | — | Vault ref. |
| `--text` | string | one-of | — | Single non-empty text. |
| `--batch` | path | one-of | — | JSONL of texts. Exactly one of `--text`/`--batch`. |
| `--idempotent` | bool | no | `true` | Accepts bare flag or `true/false`; non-idempotent ingest is rejected. |

Side effects: writes content entries + ledger entries, measures through active slots. Prints ingest result.

#### A.2.7 `anchor <vault> <cx_id> --kind <k> --value <v> [--confidence <f>] [--source <s>]` (`cmd/ingest.rs`)
Positional `<vault> <cx_id>`. `--kind`, `--value` required (strings). `--confidence` f32 finite, in `[0,1]`. `--source` string. Writes an anchor record + ledger entry.

#### A.2.8 `measure <vault> --text <s>` (`cmd/ingest.rs`)
`--text` required non-empty. Measures text through all active slots; no content entry created. Prints per-slot vectors.

#### A.2.9 `search <vault> <query> [...]` (`cmd/search.rs`)
| Flag | Type | Req | Default | Description |
|---|---|---|---|---|
| `<vault> <query>` (pos) | string | yes | — | Vault ref + non-empty query. |
| `--k` | usize | no | `10` | Result count (> 0). |
| `--fusion` | enum | no | `rrf` | `rrf, weighted-rrf, single-lens, kernel-first, pipeline`. |
| `--guard` | enum | no | `off` | `off, in-region`. |
| `--explain` | bool | no | `false` | Include scoring details. |
| `--provenance` / `--no-provenance` | bool | no | `true` | Include/exclude provenance. |
| `--fresh` / `--stale-ok` | flag | no | `--fresh` | Mutually exclusive; freshness of derived data. |
| `--filter` | json | no | — | Structured predicate. |

Queries sextant; prints scored results; may record a search ledger entry.

#### A.2.10 `kernel-answer <vault> <query> [--anchor <kind>] [--explain]` (`cmd/search.rs`)
Single authoritative kernel answer. `--anchor` string, `--explain` bool. Prints answer + optional explanation.

#### A.2.11 Intelligence commands (`cmd/intelligence.rs`)
- `bits <vault> <anchor-kind> [--explain]` — information bits for an anchor kind.
- `kernel <vault> [--anchor <kind>] [--rebuild]` — build/rebuild the kernel index; persists kernel state.
- `guard <vault> <calibrate|check|generate> [args]` — `calibrate` (`--domain`, `--set <jsonl>`, `--target-far <f32>`); `check` (`--cx <id>`, `--identity-cx`); `generate` (`--candidate-text`, `--identity-cx`). Subcommand/vault order is flexible.
- `abundance <vault>` — abundance metrics; no flags.
- `propose-lens <vault> --anchor <kind>` — proposes a lens candidate for an anchor.

#### A.2.12 Provenance commands (`cmd/provenance.rs`)
- `provenance <vault> <cx_id>` — lineage `{cx_id, ingest_seq, ledger_chain_hash, lens_measures, anchors}`; verifies chain for the entry.
- `verify-chain <vault> [--from <seq>] [--to <seq>]` — verifies the ledger hash chain over `[from,to]` (defaults `from=0`, `to=head`). Prints `{status, checked, break_at?}`. Exits error on broken/corrupt. (Distinct from the legacy `verify-chain --vault/--ledger --range` form in §A.4.)
- `reproduce <vault> <answer_id>` — re-derives an answer, prints `{bit_parity, original_hash, reproduced_hash}`; errors if parity false.
- `anneal-status <vault>` — annealing status report.

#### A.2.13 `lens add` / `lens list` (`cmd/lens.rs` and `lens_commands.rs`)
- `lens add --manifest <m.json> [--home <dir>]` — registers a manifest into `{home}/lenses/registry.json` (dedup by `lens_id`).
- `lens list [--home <dir>]` — lists registered lenses.
- `lens explain --manifest <m> [--input <s>] [--repeat <n>]` — profiles a lens (times `measure()`, reports vector norm/estimates).

`--home` falls back to `$CALYX_HOME`.

### A.3 `readback` family — structured (`cmd/readback.rs`) and legacy (`dispatch.rs`)

`readback` is a large diagnostic family that prints "source-of-truth bytes or listings for manual FSV inspection." Matched by slice patterns in `dispatch.rs` and `cmd/readback.rs`. Major forms (exhaustive list from `usage::usage()`):

| `readback` form | Key flags | Purpose |
|---|---|---|
| `--hex <file>` | — | Hex-dump a file. |
| `--vault-tree <dir>` | — | Print vault directory tree. |
| `--cf <name> --vault <dir> [--seq <n>]` | `--cf`, `--vault` | Read a CF row / ledger seq. |
| `--cf <name> --level <dir>` | `--cf`, `--level` | Read an SST level dir. |
| `--wal --vault <dir>` | `--wal`, `--vault` | Read WAL segments. |
| `--vault <d> --verify-against <sqlite.db>` | | Dual-write verify (leapable). |
| `--vault <d> --show-manifest` | | Shadow manifest. |
| `vault-manifest --field <name> --vault <d>` | | One manifest field. |
| `temporal_search --explain --clock-fixed <s> --tz-offset <s>` | | Temporal-search trace. |
| `dedup-check --vault --cx-id --slot --tau --near-cos --distinct-cos --vault-id --salt` | | Dedup decision. |
| `kernel-health --root <d> --kernel-id <cx>` | | Kernel health. |
| `recurrence-series --vault --cx-id` ; `periodic-recall --vault (--hour\|--day)` | | Recurrence. |
| `oracle_self_consistency \| oracle_sufficiency \| oracle_predict \| oracle_expand \| reverse_query \| super_intelligence` (`--vault --domain/--fixture --vault-id --salt [--depth]`) | | Oracle readbacks (`oracle_readback.rs`; `is_topic`). |
| `temporal-log-recurrence --log <csv> --vault --out --rows --expected-cadence-secs --confidence-ceiling` | | Log recurrence. |
| `assay-report \| temporal-cross-term \| kernel-weights \| kernel-window \| ward-novelty \| compression-ratio \| compression-report \| anneal-schedule --artifact <json> [--field <path>]` | | PH42/PH59 artifact readback (`ph42_readback.rs`; `is_topic`; schema error code `CALYX_PH42_ARTIFACT_SCHEMA`). |
| `config <tripwire\|budget> --vault <d>` | | Config readback. |
| `ledger --kind Anneal --action <…> --last <n> --vault <d>` | | Anneal ledger. |
| `anneal mistakes --vault --last <n>` | | Anneal mistakes. |
| `dedup-audit --vault --cx-id` ; `dedup-undo --vault --token <json>` ; `cx-list --vault` | | Dedup audit. |
| `time-index --vault` ; `as-of --vault --t-millis <ms>` ; `time-prediction --vault --cx-id --confidence-ceiling <f>` | | Time-travel. |
| `trigger-audit <sub-id> --vault` ; `trigger-fired --vault` | | Triggers. |

### A.4 Legacy ledger / FSV / ops commands (`dispatch.rs`)

All use slice-pattern matching; `--vault`/`--ledger` are repeated explicit flags.

| Command | Args | Purpose / side effects |
|---|---|---|
| `merkle-root (--ledger <d> \| --vault <d>) --range <a..b>` | also `CALYX_LEDGER_DIR=<d> calyx merkle-root --range <a..b>` | Merkle root over a seq range (`merkle.rs`). |
| `verify-chain (--ledger <d> \| --vault <d>) --range <a..b>` | | Hash-chain verify over `a..b` (`verify.rs`). |
| `verify-restore --vault <d> [--json]` | | PH67 byte-level restore verify (§A.6). |
| `scan --cf ledger --vault <d>` ; `ledger-tail --vault --last <n>` | | Scan/tail ledger (`scan.rs`). |
| `get-provenance --vault --cx <id>` ; `get-answer-trace --vault --answer <id-or-hex>` ; `audit --vault --kind <k>` | | Provenance (`provenance.rs`). |
| `compact --vault --cf <n>` ; `compact-watch --vault --duration <30s\|500ms>` | | Compaction (`ops.rs`). |
| `soak --vault --ops <n> --threads <n>` ; `resource-status --vault [--metrics]` | | Soak / resource status (`ops.rs`, `resource_status.rs`). |
| `resource-drill --vault --ops --value-bytes --memtable-cap --pin-max-age-ms` | | Memtable/pin drill (`resource_drill.rs`). |
| `tier --vault --cf --output <hot\|cold>` | | Tier move. |
| `vault-demo \| arrow-demo \| cf-demo \| mvcc-demo --vault <d>` | | Demos (`ops.rs`, `fsv.rs`). |
| `wal-drill --vault --records <n>` ; `wal-replay <wal-dir>` ; `wal-batch-demo --vault --requests <n>` | | WAL FSV (`fsv.rs`, `ops.rs`). |
| `crash-drill --vault --point <before-wal-fsync\|after-wal-before-commit\|after-commit-before-manifest> [--pause-ms <n>]` | | Crash injection (`crash.rs`, `CrashPoint::parse`). |
| `recover --vault <d>` ; `open-check --vault --index <n>` ; `corrupt-shard --vault --cf --byte-offset <n>` | | Recovery / corruption FSV (`crash.rs`, `fsv.rs`). |
| `ward tau --slot <n> --vault <d>` | | Ward τ readback. |

### A.5 Domain command groups (legacy dispatcher)

Each group has a `run(topic/mode, rest)` dispatcher. Defaults shown where the handler sets them. All emit JSON; validators reject unknown flags fail-closed.

#### `navigate <mode>` (`navigate/`) — rehydrates a `SearchEngine` from `--spec <json>`; all modes accept `--out <path>` (writes a BLAKE3-digested SoT file).
| Mode | Required flags | Optional |
|---|---|---|
| `neighbors` | `--spec --cx --slot --k` | `--out` |
| `define` | `--spec --cx --slot --k` | `--out` |
| `agree` | `--spec --anchor --k` | `--slots <a,b>` `--out` |
| `disagree` | `--spec --anchor --k` | `--slots <a,b>` `--out` |
| `traverse` | `--spec --anchor --direction <forward\|backward\|both> --hops <1-10>` | `--out` |
| `skills` | `--spec` | `--min-cluster-size`(2) `--min-samples`(1) `--max-constellations` `--slots` `--allow-single` `--out` |
| `search-skill` | `--spec --skill --slot --k --vec <a,b>` | `--text`("navigate-search-skill") `--min-cluster-size`(2) `--min-samples`(1) `--out` |

#### `sextant <topic>` (`sextant_commands.rs`)
- `recall-validate --corpus-jsonl --queries-jsonl --qrels --metrics-dir --vault [--query-limit 50] [--k 10] [--min-delta 0.15] [--vault-id] [--salt calyx-ph70-sextant-recall]`.
- `diskann-validate --root [--mode happy|empty|dim-mismatch|truncated|missing-raw] [--nodes 1000] [--dim 64] [--queries 128] [--k 10] [--beamwidth 32] [--ef-search 128] [--rescore-k 64]` — writes graph/raw/metrics under `--root`.

#### `media <topic>` (`media_commands.rs`)
- `image-validate --samples --metrics-dir --vault [--min-image-bits 0.05] [--min-cross-modal-bits 0.05] [--k 3] [--vault-id] [--salt calyx-ph70-media-image]`.
- `emotion-validate --samples --metrics-dir --vault [--min-bits 0.05] [--k 3] [--vault-id] [--salt calyx-ph70-media-emotion]`.

#### `lodestar kernel-validate` (`lodestar_commands.rs`)
`--corpora-dir --metrics-dir [--min-ratio 0.95 (in [0,1])] [--query-limit 500] [--top-k 10]`.

#### `panel status` (`panel_commands.rs`)
`[--home <dir>]` or `[--vault <dir>]` (mutually exclusive). Lists lens placement (CPU/GPU), cost, manifest path; aggregates totals.

#### `intelligence abundance --vault <dir>` (`intelligence_commands.rs`)
Reads `{vault}/intelligence/abundance.json`.

#### `summarize` (`summarize_command.rs`)
`--vault --scope <json|@file> --out` required; optional `--graph`(assoc) `--as-of <ms>` `--anchor-label` `--max-kernel-size` `--require-grounded` `--cache-ttl-secs` `--recall-top-k` `--recall-held-out` `--recall-seed` `--recall-min-ratio` `--vault-id` `--salt`(calyx-summarize-cli). Writes pretty JSON to `--out` and stdout.

#### `migrate <topic>` (`migrate/`)
| Topic | Positional | Flags |
|---|---|---|
| `vault` | `<sqlite.db> <vault.calyx>` | `--verify` `--dry-run` `--gte-lens-id <hex16>` `--gte-endpoint <url>` `--backfill-default-panel` `--offline-backfill` `--batch-size`(100). Writes `{vault}/migration.manifest.json`. |
| `backfill` | `<sqlite.db> <vault.calyx>` | `--offline-backfill` `--batch-size`(16). |
| `verify` | `<sqlite.db> <vault.calyx>` | `--require-backfill`. Prints byte-exact match / MISMATCH lines; errors if any mismatch. |
| `status` | `<vault.calyx>` | — (JSON StatusReport). |
| `readback` | `<sqlite.db> <vault.calyx> <chunk_id>` | — (decoded chunk JSON). |

#### `anneal <subcommand>` (`anneal_commands.rs`)
All emit JSON unless noted; `--last` must be > 0 when present.

| Subcommand | Required | Optional / notes |
|---|---|---|
| `status --health --vault` | `--health --vault` | plain-text health rows from SST/WAL. |
| `status --faults --last <n> --vault` | all three | plain-text FaultEvent rows; flag order flexible. |
| `replay-status --vault` | `--vault` | replay snapshot. |
| `head-status --kind --vault` | both | online head params. |
| `bandit-status --key --vault` | both | bandit arm state. |
| `ab-log --vault` | `--vault` | `--last`(5). A/B ledger. |
| `soak --vault --corpus-jsonl --metrics-dir` | three | `--queries`(1000000) `--sample-interval`(10000) `--min-docs`(50000) `--vault-id` `--salt`. Ingests + runs seeded queries; writes metrics-dir. |
| `soak-report --vault` | `--vault` | `--last`(1). |
| `autotune-report --scope <forge\|index\|loom\|storage> --cache --vault` | three | `--slot`(required iff scope=index; forbidden for storage/loom) `--last`(5). |
| `intelligence-report --fixture` | `--fixture` | `--vault` (persists `cf/anneal_report` + gradient snapshot when present). |
| `growth-curve --vault` | `--vault` | `--last`(20); ASCII plot. |
| `goodhart-check --fixture --vault --vault-id --salt` | all four | records ledger result; updates `goodhart_state.toml`. |
| `deficit-map --anchor --fixture` | both | `--threshold <bits>` (default `DEFAULT_DEFICIT_THRESHOLD_BITS`). |
| `propose-preview --anchor --deficit --corpus` | all three | synthesizes candidate. |
| `lens-proposal-log (--fixture XOR --vault)` | one of | `--last`(5). |
| `propose-lens-run --fixture` | `--fixture` | strict positional. |
| `frozen-guard-report --artifact` | `--artifact` | counts ok/violation/new. |
| `regression-report --artifact` | `--artifact` | regression rate. |

#### `leapable <topic>` (`leapable/`) — SQLite↔Calyx shadow/migration FSV; all emit JSON.
| Topic | Required flags | Notable optional |
|---|---|---|
| `dual-write` | `--sqlite --calyx` | `--inject-shadow-failure`. Writes `DUAL_WRITE_RECEIPTS.jsonl`. |
| `read-flip` | `--sqlite --calyx` | `--tau`(0.72) `--skip-backfill`. Flips Shadow→Calyx mode; ledger entry. |
| `remove-shadow` | `--sqlite (--calyx\|--vault) --vault-type <text\|code\|civic\|media>` | archives sqlite; sets CalyxOnly mode. |
| `ask` | `--vault` + (`--query-vector <json>` XOR `--query <text>`) | `--top-k`(5). |
| `recall-compare` | `--sqlite --calyx --queries <jsonl>` | `--top-k`(10). Writes `PARITY_REPORT.jsonl`. |
| `verify-round-trip` | `--sqlite --calyx` | `--output` `--benchmark` (needs `--queries`) `--top-k`(10). |
| `shadow-open` | `--sqlite --vault` | manifest/contract state. |
| `shadow-readback` | `--vault` | reads MANIFEST without opening. |
| `issue612-fsv` | `--baseline-latency --flipped-latency --pg-before --pg-after --out` | p99 ≤ 1.05× baseline; blake3 table-hash match across 5 tables. |
| `production-fsv <snapshot-pg\|verify-pg-unchanged\|verify-contract\|run>` | per-subcommand (see below) | — |

`production-fsv` subcommands: `snapshot-pg --vault-name --out (--pg-conn XOR --pg-dump-dir)`; `verify-pg-unchanged --before --after`; `verify-contract --vault-name --snapshot`; `run --vault --vault-name --out (--pg-conn XOR --pg-dump-dir) (--query XOR --query-vector) [--query-dim 768] [--top-k 5]`.

### A.6 `verify-restore` (PH67, CLI wrapper) — `verify_restore.rs`

`calyx verify-restore --vault <path> [--json]`. Intercepted ahead of all other matchers. Calls `calyxd::verify::verify_restore` (a read-only, zero-side-effect byte-level scan of a restored vault). Prints text (or `--json` pretty JSON). Owns its exit: **0** = chain intact AND constellations/anchors/WAL bytes all present; **1** = any verification or usage failure (prints `error: …` and the failure reasons to stderr). Usage and bad-flag errors also exit 1.

### A.7 `healthcheck` (plain deploy-health) — `healthcheck.rs`

`calyx healthcheck` (NO `--config`) is the deploy-health probe (distinct from §B.4). Flags: `--wait <secs>`(0), `--out <path>`(env `CALYX_HEALTH_LOG_PATH` or `/zfs/hot/logs/calyx-health/latest.json`), `--secret-env <path>`(env `CALYX_SECRET_ENV` or `/run/leapable/secrets/calyx.env`, mode 0400), `--calyx-home <dir>`(env `CALYX_HOME` or `/opt/calyx`), `--vault <dir>`, `--metrics-url <url>`, `--require-env <name>` (repeatable; default `HF_HUB_TOKEN`,`HF_TOKEN`). Runs checks `calyx_home`, `calyx_secret_env`, optional `calyx_vault_restore_readback` (uses `calyxd::verify::verify_restore`), optional `calyx_metrics` (scrapes `calyx_ledger_chain_verify_ok`). Writes a `HealthReport` JSON, re-reads to verify the write, retries up to `--wait` seconds. Exit **0** all pass, **1** any failure. Per-check codes include `CALYX_HEALTH_HOME_MISSING`, `CALYX_HEALTH_SECRET_ENV_MISSING`, `CALYX_HEALTH_VAULT_UNVERIFIED`, `CALYX_HEALTH_METRICS_UNREACHABLE`.

### A.8 Top-level command index

Structured (PH62): `create-vault, add-lens, retire-lens, park-lens, list-panel, profile-lens, ingest, anchor, measure, search, kernel-answer, bits, kernel, guard, abundance, propose-lens, provenance, verify-chain, reproduce, anneal-status`.
Legacy/groups: `readback, healthcheck, verify-restore, merkle-root, verify-chain (range form), scan, ledger-tail, get-provenance, get-answer-trace, audit, compact, compact-watch, soak, tier, resource-status, resource-drill, vault-demo, arrow-demo, cf-demo, mvcc-demo, wal-drill, wal-replay, wal-batch-demo, crash-drill, recover, open-check, corrupt-shard, ward, migrate, navigate, sextant, media, lodestar, lens, panel, intelligence, summarize, anneal, leapable`.

---

## Part B — `calyxd`

### B.1 Module surface (`lib.rs`)

`calyxd` compiles a binary (`main.rs` + private `startup`, `verify_loop`) over a public **library** (`lib.rs`) that is the single source of truth for: `config` (`CalyxConfig`), `cuda_probe` (T02), `vram` (T03), `health` (T04), `metrics` (the `/metrics` surface, PH66 T03), `mcp_server` (T05 loopback MCP), `server` (the `/metrics` HTTP listener), `verify` (PH67 `verify-restore`), and `error` (the `CALYX_DAEMON_*` taxonomy).

### B.2 Invocation and arg parsing (`main.rs`)

`fn main()` is `#[tokio::main]`; arg parsing is hand-rolled (`parse_args`). USAGE:

```
calyxd (--vault <dir> | --ledger <dir>)... [--bind <loopback-addr:port>] [--interval-secs <n>] [--once]
calyxd --config <calyx.toml> --validate-config
```

| Flag | Type | Default | Description |
|---|---|---|---|
| `--vault <dir>` | path (repeatable) | — | Aster vault directory to chain-verify. |
| `--ledger <dir>` | path (repeatable) | — | Standalone directory ledger to chain-verify. |
| `--bind <addr>` | SocketAddr | `127.0.0.1:7700` | Loopback listen address (invalid value → `CALYX_DAEMON_CONFIG_INVALID`). |
| `--interval-secs <n>` | u64 | `60` | Seconds between verify cycles, min 1 (0 → config-invalid). |
| `--once` | flag | false | One verify cycle, print metrics text, exit. |
| `--config <path>` | path | — | A `calyx.toml`; presence (without `--validate-config`) boots **server mode**. |
| `--validate-config` | flag | false | Parse+validate `--config`, print it (no secrets), exit. |
| `--audit-vram` | flag | false | With `--config`: CUDA preflight + NVML VRAM audit (JSON), exit. |

If `--validate-config`/`--audit-vram` are set with no `--config`, the path defaults to `calyx.toml`. Without `--validate-config`/`--config`, at least one `--vault`/`--ledger` is required (else `CALYX_DAEMON_CONFIG_INVALID`). Arg-parse failure exits **2** with USAGE; runtime failure in the simple chain-verify path exits **2**; server-mode fatals exit **1** (`fatal()` in `startup.rs`).

Three run modes: (1) `--validate-config` → `startup::validate_config`; (2) `--config` present → `startup::run_server`; (3) otherwise → `run()` (the plain chain-verify loop + `/metrics`).

### B.3 Startup sequence (server mode, `startup::run_server`) — probes T02–T05

The daemon **fails loud** at startup — no silent CPU fallback. `run_server` order (each `Err` → `fatal()` exit 1):

1. **Config load** — `CalyxConfig::from_file(path)` (T01; see B.6).
2. **T02 CUDA probe** — `cuda_probe::probe_cuda_device()`. With `--features cuda` calls `calyx_forge::init_cuda(0,false)` and captures `CudaDeviceInfo {device_name, vram_total_mib, compute_cap}`. Without the feature, or on any init failure, returns `DeviceUnavailable` (`CALYX_FORGE_DEVICE_UNAVAILABLE`). Env `CALYX_FORCE_CUDA_FAIL=1` (exact string `"1"`) forces the failure path for FSV.
3. **T03 VRAM budget** — `build_vram_budget` → `NvmlVramUsage::init()` (loads `libnvidia-ml.so.1`) + `VramBudget::from_config(cfg.vram_budget_mib, &device, nvml)`, then `startup_vram_audit()`. Reads live device usage via **NVML** (not `cudaMemGetInfo`). Fails closed (`CALYX_FORGE_VRAM_BUDGET`) if budget = 0, budget > device board total, or resident footprint already > budget (error names the TEI endpoints `:8088/:8089/:8090`). `--audit-vram` prints the `VramAuditReport {tei_used_mib, calyx_budget_mib, device_total_mib}` JSON and exits 0 here.
4. **Vault read-back** — `open_vault_for_startup(vault_path)` calls `verify::verify_restore`; a present-but-unverified vault → `DaemonError::health_failed` (`CALYX_DAEMON_HEALTH_FAIL`). `vault_path` is `cfg.vault_path_resolved()` (`$CALYX_HOME` expanded).
5. **First chain-verify cycle (synchronous)** — `ChainVerifyMetrics::new`, then `run_cycle(&[target], &chain)` so a scrape never observes an unverified gauge. `CalyxMetrics::new` then composes the full surface; `refresh_zfs_metrics` populates ZFS gauges. `--once` prints the encoded text and exits here.
6. **T05/metrics bind + signals** — `MetricsServer::bind(cfg.bind_addr, surface)` (loopback-only, `CALYX_DAEMON_BIND_FAILED`); `install_signal_handlers` wires SIGINT/SIGTERM (unix) or Ctrl-C (non-unix) → `CancellationToken`.
7. **T04 healthcheck** — `run_healthcheck(&cfg)` + `write_health_result` to `health_log_path`; if not pass, prints `CALYX_DAEMON_HEALTH_FAIL` and exits **1** (listener will not accept).
8. **Loops** — `spawn_loop` (periodic chain-verify, `VERIFY_INTERVAL_SECS = 60`) + `spawn_zfs_metrics_loop`, then `server.run(cancel_token)` blocks. Clean shutdown writes `write_shutdown_status` (`{"status":"shutdown",…}`).

The plain `run()` path (no `--config`) skips T02–T04: it validates each target is a directory, runs one `run_cycle`, then either prints (`--once`) or binds `/metrics` and spawns the loops.

#### Chain-verify loop (`verify_loop.rs`)
`VerifyTarget {kind: Vault|LedgerDir, path}`; `validate()` requires the path be an existing directory (else `CALYX_DAEMON_CONFIG_INVALID`). Each cycle re-opens the store fresh (`AsterLedgerCfStore::open` / `DirectoryLedgerStore::open`), verifies `0..head` via `calyx_ledger::verify_chain`, and records a `VerifyOutcome` (`Intact{entries}`, `Broken{at_seq}`, `Corrupt{at_seq,reason}`, `Error{detail}`). Non-intact outcomes log the relevant `CALYX_LEDGER_*` code; the `_ok` gauge goes to 0 (a broken chain is never an exit — it is the alert).

### B.4 Health probe (T04, `health.rs`)

`run_healthcheck(cfg) -> CalyxHealthResult` runs two probes and never panics/returns Err (fail-closed into the record): (1) CUDA init + VRAM budget honored against live NVML; (2) a real Aster vault read-back via `verify::verify_restore`. First failure wins for `error_code`; per-subsystem fields still record each outcome.

`CalyxHealthResult` fields: `status` (`"pass"`/`"fail"`), `timestamp_utc` (ISO-8601 `Z`, computed dependency-free via Howard Hinnant's `civil_from_days`), `cuda_device: Option<String>`, `vram_budget_mib: u32`, `vault_read_ok: bool`, `error_code: Option<String>`, `error_detail: Option<String>`. Written atomically (temp-sibling + rename, same-FS to avoid EXDEV) to `health_log_path`. `write_shutdown_status` writes `{status:"shutdown", timestamp_utc}`. `run_with_wait(wait_secs, …)` re-probes at 1-second intervals up to `wait_secs+1` attempts, stopping on first pass.

The CLI front-ends are `calyx healthcheck --config <toml>` (`healthcheck_daemon.rs`; exit **0** healthy / **1** ran-but-unhealthy / **2** cannot-run) — flags `--config` (required), `--wait <secs>` (default `healthcheck_timeout_secs`), `--out <path>` (default `health_log_path`).

### B.5 Prometheus `/metrics` surface

#### Transport (`server.rs`)
Loopback-only `TcpListener` (non-loopback bind → `CALYX_DAEMON_BIND_FAILED`), thread-per-connection, panic-isolated. Minimal HTTP/1.1: `GET /metrics` → 200 text `version=0.0.4`; other GET paths → 404; other methods → 405; unreadable head → 400. Reads the full request head (avoids TCP RST). `CancellationToken` stops the accept loop; drains up to 5s.

#### Surface (`metrics.rs` + `metrics/calyx.rs`)
`CalyxMetrics::encode_text()` concatenates the chain-verify family then the PH66 T03 families. Exhaustive metric list:

| Metric | Type | Labels | Meaning |
|---|---|---|---|
| `calyx_ledger_chain_verify_ok` | gauge | vault | 1 iff last verify proved chain intact (else 0). |
| `calyx_ledger_chain_verify_last_run_timestamp_seconds` | gauge | vault | Unix ts of last verify run. |
| `calyx_ledger_chain_verify_entries` | gauge | vault | Entries proven intact (0 unless intact). |
| `calyx_ledger_chain_verify_runs_total` | counter | vault, outcome | Runs by outcome (`intact\|broken\|corrupt\|error`). |
| `calyx_ingest_duration_seconds` | histogram | vault | Ingest latency (buckets 0.001…10s). |
| `calyx_ingest_total` | counter | vault, status | Ingests by `ok\|err`. |
| `calyx_search_duration_seconds` | histogram | vault, strategy | Search latency by strategy. |
| `calyx_search_recall_tripwire` | gauge | vault | 1 healthy / 0 tripped; pre-initialized to 1. |
| `calyx_search_total` | counter | vault, strategy, status | Searches by strategy+status. |
| `calyx_guard_far` / `calyx_guard_frr` | gauge | vault, slot | Guard false-accept / false-reject rate. |
| `calyx_assay_n_eff` | gauge | vault, panel | DDA effective sample size. |
| `calyx_kernel_recall_ratio` | gauge | vault, scope | Kernel recall vs brute force. |
| `calyx_anneal_ab_variant_total` | counter | experiment, variant | A/B exposures. |
| `calyx_anneal_ab_improvement_ratio` | gauge | experiment | A/B improvement ratio. |
| `calyx_vram_budget_used_mib` / `calyx_vram_budget_limit_mib` | gauge | — | VRAM used / ceiling (MiB). |
| `calyx_hazard_<id>` (×25) | gauge | hazard | One per PH59 hazard, 1=tripwire firing. |
| `calyx_zfs_pool_healthy` | gauge | pool | 1 when `zpool status -x` healthy. |
| `calyx_zfs_cksum_errors_total` | gauge | pool | CKSUM errors from `zpool status -v`. |
| `calyx_zfs_scrub_age_seconds` | gauge | pool | Age of last/running scrub. |
| `calyx_zfs_dataset_checksum_enabled` | gauge | dataset | 1 when `zfs get checksum` ≠ off. |

`strategy` ∈ `single_lens, rrf, weighted_rrf, sparse`. Search strategy/vault/ingest series are pre-initialized; guard/assay/kernel/anneal Vec families appear on first observation. The 25 hazard ids (`metrics/hazards.rs`, `HAZARD_IDS`): `compaction_storm, flush_stall, tombstone_buildup, fsync_spike, wal_bloat, mvcc_version_pileup, vram_oom, heap_oom, nan_propagation, quant_drift, codebook_staleness, ann_corruption, hot_shard_skew, lock_contention, cache_stampede, slow_lens_hol, disk_full, arc_thrash, clock_skew, anneal_thrash, panel_explosion, secret_leakage, nondeterminism, whole_host_loss, upgrade_skew`. Default ZFS datasets: `hotpool/calyx, archive/calyx, archive/calyx-restic` (`ZFS_SCRUB_MAX_AGE_SECONDS = 40 days`). Duplicate metric registration panics at init.

### B.6 Config loading (T01, `config.rs`)

`CalyxConfig` is the authoritative TOML config. Constructed only via `from_file`/`from_toml_str`, both of which run `validate()` (fail-closed). `#[serde(deny_unknown_fields)]` — a typo'd key → `CALYX_DAEMON_CONFIG_INVALID`. Secrets never live here.

| Key | Type | Req | Default | Validation |
|---|---|---|---|---|
| `bind_addr` | SocketAddr | no | `127.0.0.1:7700` | Must be loopback, else `CALYX_DAEMON_BIND_FAILED`. |
| `vault_path` | PathBuf | **yes** | — | `$CALYX_HOME`/`${CALYX_HOME}` expanded by `vault_path_resolved()`. |
| `vram_budget_mib` | u32 | **yes** | — | `1..=30000` (`VRAM_BUDGET_MIB_CEILING`), else `CALYX_FORGE_VRAM_BUDGET`. |
| `log_dir` | PathBuf | **yes** | — | — |
| `health_log_path` | PathBuf | no | `/zfs/hot/logs/calyx-health/latest.json` | — |
| `tei_endpoints` | Vec<String> | no | `[]` | Documents `:8088/:8089/:8090`. |
| `healthcheck_timeout_secs` | u32 | no | `30` | — |

A TOML syntax error or missing required key → `CALYX_DAEMON_CONFIG_INVALID` naming the cause. See [03_configuration.md](03_configuration.md).

### B.7 MCP-over-socket transport (T05, `mcp_server.rs`)

`CalyxMcpServer` is **transport only** — it frames length-prefixed JSON-RPC on a loopback TCP socket and hands each decoded request to a shared `calyx_mcp::McpServer` (which owns the protocol, per-tool panic isolation, and the `CalyxError`→`-32000` mapping). Uses std threads (the workspace is synchronous). Wire format: 4-byte big-endian `u32` length prefix + that many UTF-8 JSON bytes. Constants: `MAX_FRAME_BYTES = 4 MiB`, `IO_TIMEOUT = 5s`, `DRAIN_TIMEOUT = 5s`.

Fail-closed posture: non-loopback bind → `CALYX_DAEMON_BIND_FAILED` (never starts); oversized/zero/truncated frame → `CALYX_DAEMON_FRAME_INVALID` (connection closed); malformed JSON inside a valid frame → a JSON-RPC error reply (`CALYX_MCP_JSONRPC_INVALID`) and the connection stays open; a panicking handler → `CALYX_DAEMON_CONN_PANIC` logged, accept loop survives. Notifications (no `id`) get no reply. `ShutdownHandle::shutdown()` sets the flag then self-connects to wake the blocked `accept()`.

> Note: `mcp_server` is library API for T05 (loopback dispatch); it registers no production tools (that is PH63/T06) and `main.rs`/`startup.rs` do not currently wire it into the running daemon — the binary runs the `/metrics` listener. See [19_mcp_api_tools_reference.md](19_mcp_api_tools_reference.md).

### B.8 `verify-restore` (PH67, `verify.rs`)

`verify_restore(vault_path) -> Result<VerifyRestoreReport, DaemonError>` opens a restored vault **read-only with zero write side-effects** (no `CfRouter::open`/`DurableVault::open`; no dir/WAL creation, no torn-tail truncation). Every count comes from physically scanning SST files + WAL frames; the ledger walk recomputes every hash link from genesis via `calyx_ledger::verify_chain`. Rebuildable index dirs (`ann, kernel, guard`) may be absent (logged, never a failure).

`VerifyRestoreReport`: `vault_path, constellation_count, anchor_count, ledger_entry_count, ledger_tip_hash` (hex), `chain_intact, wal_bytes_present, first_cx_id: Option<String>, error: Option<String>`. `success()` ⇔ no error AND chain intact AND constellation/anchor/WAL-bytes all > 0. `failure_reasons()` names every unmet criterion (or the single scan error). Fail-closed: missing/non-dir path or a dir with neither `cf/` nor `wal/` → `CALYX_DAEMON_CONFIG_INVALID`; a broken/corrupt chain → `Ok(report)` with `error` set to the exact `CALYX_LEDGER_*` code (never a silent zero-fill). The first constellation is fully decoded (base row + key match + every listed slot column present and decodable). This function backs the CLI `verify-restore`, the daemon startup read-back, and the T04 health vault probe.

### B.9 Error taxonomy (`error.rs`) — `CALYX_DAEMON_*` roots

`DaemonError`'s `Display` always renders `<code>: <detail> (remediation: <hint>)`.

| Variant | `code()` | Remediation summary |
|---|---|---|
| `BindFailed` | `CALYX_DAEMON_BIND_FAILED` | Set `bind_addr` to a loopback address. |
| `ConfigInvalid` | `CALYX_DAEMON_CONFIG_INVALID` | Fix the named `calyx.toml` key / CLI arg. |
| `VramBudget` | `CALYX_FORGE_VRAM_BUDGET` | Lower `vram_budget_mib` or free GPU memory. |
| `DeviceUnavailable` | `CALYX_FORGE_DEVICE_UNAVAILABLE` | Ensure CUDA GPU + driver + `--features cuda`; server mode requires a working GPU. |
| `HealthFailed` | `CALYX_DAEMON_HEALTH_FAIL` | Inspect the failing probe (CUDA/VRAM/vault read), fix, re-run `calyx healthcheck`. |

Transport-local string codes (not `DaemonError` variants): `CALYX_DAEMON_FRAME_INVALID`, `CALYX_DAEMON_CONN_PANIC` (`mcp_server.rs`, `server.rs`). The two `CALYX_FORGE_*` codes are reused from the Forge subsystem; the daemon-owned roots are the three `CALYX_DAEMON_*` codes (plus the `CALYX_DAEMON_FRAME_INVALID`/`CALYX_DAEMON_CONN_PANIC` transport sentinels).

---

## Gaps / not covered

- **MCP transport not wired into the running daemon.** `mcp_server.rs` (T05) is a complete, tested library surface but neither `main.rs` nor `startup.rs` constructs/runs a `CalyxMcpServer`; the live daemon serves only `/metrics`. Production tool registration is deferred (PH63/T06 per the module docs).
- **CUDA path is feature-gated.** `cuda_probe::probe_real_device` only does a real probe with `--features cuda`; default builds fail loud at startup in server mode. The repository default build (`default = []`) cannot serve.
- **CLI metric recording is mostly dormant in the daemon.** `CalyxMetrics`' `observe_ingest`/`observe_search`/guard/assay/anneal setters are public API "driven later by the ingest/search dispatch paths" (`lib.rs`); the running `calyxd` only updates the chain-verify, VRAM, ZFS, and hazard families.
- **CLI command count is large and partly diagnostic/FSV.** Many `readback`, `*-demo`, `*-drill`, and `validate` subcommands are FSV/diagnostic harnesses rather than production operations; flag-level defaults for some deep readback handlers (e.g. individual oracle/ph42 sub-fixtures) are documented at the dispatcher level. No `todo!()`/stub commands were observed in the dispatch paths read.
