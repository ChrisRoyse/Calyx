# 03. Configuration

This document describes **only what the Calyx source actually does**. Every key,
variable, default, and rule below is traced to the file that reads or defines it.
Anything not derivable from source is marked "Not determined from source".

## Source files covered

- `C:\code\Calyx\.env.example` (and gitignored `.env`)
- `C:\code\Calyx\rust-toolchain.toml`
- `C:\code\Calyx\.cargo\config.toml`
- `C:\code\Calyx\infra\aiwonder\calyx.toml` (reference daemon config)
- `C:\code\Calyx\crates\calyxd\src\config.rs` (the `CalyxConfig` struct + validation)
- `C:\code\Calyx\crates\calyxd\src\cuda_probe.rs`
- `C:\code\Calyx\crates\calyx-forge\src\vram\budget.rs`
- `C:\code\Calyx\crates\calyx-forge\src\vram\yield_policy.rs`
- `C:\code\Calyx\crates\calyx-forge\build.rs`
- `C:\code\Calyx\crates\calyx-registry\src\profile\gating.rs`
- `C:\code\Calyx\crates\calyx-registry\src\runtime\common.rs`
- `C:\code\Calyx\crates\calyx-registry\src\runtime\onnx\dynamic_ort.rs`
- `C:\code\Calyx\crates\calyx-registry\src\runtime\adapters\lens.rs`
- `C:\code\Calyx\crates\calyx-registry\src\spec.rs`
- `C:\code\Calyx\crates\calyx-assay\src\contract.rs`
- `C:\code\Calyx\crates\calyx-aster\src\gc\snapshot_gc\reclaimer.rs`
- `C:\code\Calyx\crates\calyx-aster\src\compaction\tiering.rs`
- `C:\code\Calyx\crates\calyx-cli\src\healthcheck.rs`
- `C:\code\Calyx\crates\calyx-cli\src\merkle.rs`
- `C:\code\Calyx\crates\calyx-cli\src\ops.rs`
- `C:\code\Calyx\crates\calyx-cli\src\lens_commands.rs`
- `C:\code\Calyx\crates\calyx-cli\src\panel_commands.rs`
- `C:\code\Calyx\crates\calyx-ward\src\ort_runtime.rs`
- `C:\code\Calyx\crates\calyx-anneal\src\propose\registry_hot_add.rs`
- `C:\code\Calyx\crates\calyx-anneal\src\budget.rs`, `j\j_composite.rs`, `j\goodhart.rs`, `tripwire.rs`

---

## 1. Config files and formats

### 1.1 `.env` / `.env.example` (repo root)

`.env.example` is **tracked**; `.env` is gitignored (per the header comment). The
template carries **names only, no secret values** (Doctrine §8c: never commit a
secret value). The Rust code does **not** parse `.env` itself — these variables are
expected to be exported into the process environment (e.g. by a shell or
Infisical-rendered `calyx.env`). The file is operational documentation for the
aiwonder deployment.

Variables declared in `.env.example` (`C:\code\Calyx\.env.example`):

| Variable | Example value in template | Purpose (per template comments) |
|---|---|---|
| `AIWONDER_VPN_SERVER` | (blank) | Cisco AnyConnect VPN server; must be up before SSH |
| `AIWONDER_VPN_USER` | (blank) | VPN username |
| `AIWONDER_VPN_PASSWORD` | (blank) | VPN password (secret) |
| `AIWONDER_HOST` | `aiwonder.mst.com` | SSH host for aiwonder |
| `AIWONDER_IP` | (blank) | SSH IP |
| `AIWONDER_SSH_PORT` | `22` | SSH port |
| `AIWONDER_USER` | `croyse` | SSH user |
| `AIWONDER_SSH_PASSWORD` | (blank) | SSH password (secret) |
| `AIWONDER_SUDO_PASSWORD` | (blank) | sudo password (secret) |
| `CALYX_HOME` | `/home/croyse/calyx` | Self-contained Calyx root on aiwonder (also read by the code — see §4) |
| `CALYX_HOT_DATASET` | `hotpool/calyx` | ZFS hot dataset name |
| `CALYX_COLD_DATASET` | `archive/calyx` | ZFS cold/archive dataset name |
| `CALYX_HOT_PATH` | `/zfs/hot/calyx` | ZFS hot mount path |
| `CALYX_COLD_PATH` | `/zfs/archive/calyx` | ZFS archive mount path |
| `CALYX_CARGO_TARGET_DIR` | `/home/croyse/calyx/target` | Shared cargo target dir |
| `CALYX_RUST_ENV` | `/home/croyse/.cargo/env` | Path to rustup `env` script |
| `CALYX_ORT_LIB_DIR` | `/home/croyse/calyx/vendor/onnxruntime-v1.26.0/build/Linux/Release` | aiwonder-built ONNX Runtime CUDA 13 library directory |
| `ORT_DYLIB_PATH` | `/home/croyse/calyx/vendor/onnxruntime-v1.26.0/build/Linux/Release/libonnxruntime.so` | Dynamic ONNX Runtime library loaded by `ort/load-dynamic` |
| `AIWONDER_TEI_GENERAL` | `http://127.0.0.1:8088` | Resident TEI general embeddings server |
| `AIWONDER_TEI_RERANKER` | `http://127.0.0.1:8089` | Resident TEI reranker server |
| `AIWONDER_TEI_LEGAL` | `http://127.0.0.1:8090` | Resident TEI legal embeddings server |
| `AIWONDER_PROMETHEUS` | `http://127.0.0.1:9090` | Prometheus endpoint |
| `HF_TOKEN` | (blank) | Hugging Face token (secret) |
| `HF_HUB_TOKEN` | (blank) | Hugging Face Hub token (secret) |
| `HF_HOME` | `/home/croyse/calyx/.hf-cache` | HF cache home (also read by the code — see §4) |
| `CALYX_REPO` | `chrisroyse/calyx-dev` | GitHub dev-state repo |

Note: most of these names (VPN/SSH/dataset/TEI/Prometheus/`CALYX_REPO`) are **not**
read by any `std::env::var` call in `crates/` — they are consumed by deployment
scripts, not the Rust binaries. The Rust code reads only `CALYX_HOME`, `HF_HOME`,
`HF_HUB_TOKEN`, `HF_TOKEN`, and `ORT_DYLIB_PATH` from this set (see §4).

### 1.2 `infra/aiwonder/calyx.toml` — the daemon config (TOML)

This is the single authoritative runtime config for `calyxd`, parsed into
`calyxd::config::CalyxConfig` (`crates\calyxd\src\config.rs`). Secrets never live
here. Validated fail-closed (see §5). Reference contents:

```toml
bind_addr = "127.0.0.1:7700"
vault_path = "$CALYX_HOME/data/vault"
vram_budget_mib = 8192
log_dir = "/zfs/hot/logs/calyx"
health_log_path = "/zfs/hot/logs/calyx-health/latest.json"
tei_endpoints = [
  "http://127.0.0.1:8088",
  "http://127.0.0.1:8089",
  "http://127.0.0.1:8090",
]
healthcheck_timeout_secs = 30
```

The file header documents that it is validated with
`cargo run -p calyxd -- --config infra/aiwonder/calyx.toml --validate-config`.
There is deliberately **no `[service]` or `[storage]` table**: `deny_unknown_fields`
would reject any unconsumed section. systemd/storage wiring lives in repo-owned unit
files instead.

### 1.3 Secondary TOML configs (parsed via `toml::from_str`)

These are parsed from caller-supplied paths (not a fixed location), each with
`#[serde(deny_unknown_fields)]` and a `validate()` step:

| Struct | File | Loader |
|---|---|---|
| `BudgetConfig` | `crates\calyx-anneal\src\budget.rs` | `read_budget_config(path)` → `toml::from_str` → `validate()` |
| `TripwireFile` | `crates\calyx-anneal\src\tripwire.rs` | `toml::from_str` |
| `JWeights` | `crates\calyx-anneal\src\j\j_composite.rs` | `toml::from_str` |
| `GoodhartState` | `crates\calyx-anneal\src\j\goodhart.rs` | `toml::from_str` then `validate_state` |
| Navigation spec (`NavSpec` et al.) | `crates\calyx-cli\src\navigate\spec.rs` | `Deserialize` with `deny_unknown_fields` (multiple structs) |

These are not loaded from a fixed env-defined path; they are tool inputs. Their
detailed field schemas are out of scope for the runtime-config tables below.

---

## 2. Every `CalyxConfig` key (daemon config)

Defined in `crates\calyxd\src\config.rs`. Construction is **only** via
`from_file` / `from_toml_str`, both of which run `validate()` before returning.

| Key | Type | Required? | Default | Valid range / rule | Description | Source |
|---|---|---|---|---|---|---|
| `bind_addr` | `SocketAddr` | optional | `127.0.0.1:7700` (`default_bind_addr`) | Must be a **loopback** IP (`127.0.0.1` or `[::1]`); else `CALYX_DAEMON_BIND_FAILED` | Address daemon listens on | `config.rs:29-31,51-52,99-104` |
| `vault_path` | `PathBuf` | **required** | none | Supports `$CALYX_HOME` / `${CALYX_HOME}` interpolation via `vault_path_resolved()` | Aster vault directory | `config.rs:55,115-120` |
| `vram_budget_mib` | `u32` | **required** | none | `1..=30000` (`VRAM_BUDGET_MIB_CEILING = 30_000`); else `CALYX_FORGE_VRAM_BUDGET` | VRAM budget for Forge, MiB | `config.rs:24,57,105-111` |
| `log_dir` | `PathBuf` | **required** | none | (none beyond presence) | Daemon log directory | `config.rs:59` |
| `health_log_path` | `PathBuf` | optional | `/zfs/hot/logs/calyx-health/latest.json` (`default_health_log_path`) | (none) | Path the healthcheck JSON is written to | `config.rs:33-35,62-63` |
| `tei_endpoints` | `Vec<String>` | optional | `[]` (serde `default`) | (none) | Documented TEI endpoints (`:8088`/`:8089`/`:8090`) | `config.rs:65-66` |
| `healthcheck_timeout_secs` | `u32` | optional | `30` (`default_healthcheck_timeout_secs`) | (none) | Healthcheck timeout, seconds | `config.rs:37-39,68-69` |

`vault_path_resolved()` substitutes the value of env var `CALYX_HOME` for
`$CALYX_HOME`/`${CALYX_HOME}`. When `CALYX_HOME` is unset, the literal path is
returned unchanged (no silent expansion to empty) — `config.rs:118-136`.

---

## 3. Other runtime-tunable defaults read from config-like sources

These are not in `CalyxConfig` but are configurable via environment variables or
hold documented constant defaults (see §4 for the env-var table).

| Constant | Value | Source |
|---|---|---|
| `DEFAULT_SOFT_CAP_BYTES` (Forge VRAM soft cap) | `12 GiB` = `12_884_901_888` | `calyx-forge\src\vram\budget.rs:13` |
| `RESERVED_HEADROOM_BYTES` | `512 MiB` | `calyx-forge\src\vram\budget.rs:17` |
| `DEFAULT_ANNEAL_VRAM_CAP_BYTES` | `2 GiB` | `calyx-forge\src\vram\yield_policy.rs:14` |
| `DEFAULT_SERVING_STREAM_PRIORITY` | `0` | `yield_policy.rs:15` |
| `DEFAULT_ANNEAL_STREAM_PRIORITY` | `-1` | `yield_policy.rs:16` |
| `DEFAULT_POWER_BACKOFF_THRESHOLD_W` | `560` | `yield_policy.rs:17` |
| `DEFAULT_ANNEAL_THROTTLE_SLEEP` | `50 ms` | `yield_policy.rs:18` |
| `DEFAULT_OOM_MAX_RETRIES` | `3` | `calyx-forge\src\vram\oom_guard.rs:15` (no env override found) |
| `DEFAULT_GC_MAX_OPS_PER_RUN` | `1_000` | `calyx-aster\src\gc\snapshot_gc\reclaimer.rs:14` |
| `DEFAULT_GC_MIN_INTERVAL_MS` | `1_000` | `reclaimer.rs:17` |
| `MIN_SIGNAL_BITS` | `0.05` | `calyx-assay\src\contract.rs:8` |
| `MAX_PAIRWISE_CORR` | `0.6` | `calyx-assay\src\contract.rs:9` |
| `DEFAULT_MAX_TOKENS` | `512` | `calyx-registry\src\runtime\common.rs:10` |

---

## 4. Environment variables read by the code

This table lists **every** `std::env::var` / `env::var_os` read in non-test source
(`crates/**/src/**/*.rs`). Test-only FSV variables (e.g. `CALYX_FSV_ROOT`,
`CALYX_ISSUE*_FSV_ROOT`, `CALYX_*_FSV_DIR`, `CALYX_CANDLE_FSV_*`,
`CALYX_FUSION_WEIGHTS_FSV_ROOT`, `CALYX_RECURRENCE_*_FSV_ROOT`, etc.) are excluded;
they only gate test harnesses and are not runtime config.

| Variable | Type/parse | Default when unset | Valid range / behavior | Read by (file:line) |
|---|---|---|---|---|
| `CALYX_HOME` | path/string | varies by caller (see below) | Interpolated into `vault_path`; used to derive vault/lens/HF-cache/tier roots | `calyxd\src\config.rs:119`; `registry\src\runtime\common.rs:16`; `aster\src\compaction\tiering.rs:170`; `anneal\src\propose\registry_hot_add.rs:236`; `cli\src\ops.rs:414`; `cli\src\lens_commands.rs:245`; `cli\src\panel_commands.rs:193`; `cli\src\healthcheck.rs:82` |
| `HF_HOME` | path | `$CALYX_HOME/.hf-cache`, else `.hf-cache` | HF / fastembed cache root | `registry\src\runtime\common.rs:13,23` |
| `HF_HUB_TOKEN` | (presence) | — | Required env name probed by `calyx healthcheck` | `cli\src\healthcheck.rs:22` (`DEFAULT_REQUIRED_ENV`) |
| `HF_TOKEN` | (presence) | — | Required env name probed by `calyx healthcheck` | `cli\src\healthcheck.rs:22` |
| `CALYX_FORGE_VRAM_BUDGET` | decimal bytes (`usize`) | `DEFAULT_SOFT_CAP_BYTES` = 12 GiB | Non-integer → fail closed with `CALYX_FORGE_VRAM_BUDGET` | `calyx-forge\src\vram\budget.rs:86,376-378` |
| `CALYX_ANNEAL_VRAM_BUDGET` | bytes (`usize`) | `DEFAULT_ANNEAL_VRAM_CAP_BYTES` = 2 GiB | Invalid → **fail closed: cap set to 0**, error logged | `calyx-forge\src\vram\yield_policy.rs:50-62` |
| `CALYX_FORCE_CUDA_FAIL` | string | (unset = real probe) | Only exact `"1"` forces CUDA probe failure (FSV injection) | `calyxd\src\cuda_probe.rs:35` |
| `CALYX_CAPABILITY_MIN_SIGNAL_BITS` | `f32` | `MIN_SIGNAL_BITS` = 0.05 | Validated: finite, `>= 0.0` | `registry\src\profile\gating.rs:32` |
| `CALYX_CAPABILITY_MAX_PAIRWISE_CORR` | `f32` | `MAX_PAIRWISE_CORR` = 0.6 | Validated: finite, in `[0.0, 1.0]` | `registry\src\profile\gating.rs:33` |
| `CALYX_ALLOW_NONCOMMERCIAL_LENSES` | string flag | (unset = denied) | Gates non-commercial lens licensing; else `CALYX_LICENSE_DENIED` | `registry\src\runtime\adapters\lens.rs:196` |
| `CALYX_GC_MAX_OPS_PER_RUN` | `usize` | `DEFAULT_GC_MAX_OPS_PER_RUN` = 1000 | Invalid → fail closed (error) | `aster\src\gc\snapshot_gc\reclaimer.rs:19,48,330` |
| `CALYX_GC_MIN_INTERVAL_MS` | `u64` | `DEFAULT_GC_MIN_INTERVAL_MS` = 1000 | Invalid → fail closed (error) | `aster\src\gc\snapshot_gc\reclaimer.rs:20,49,340` |
| `CALYX_LEDGER_DIR` | path | — (required) | Required when `--ledger` is omitted for merkle root | `cli\src\merkle.rs:18` |
| `CALYX_HEALTH_LOG_PATH` | path | `/zfs/hot/logs/calyx-health/latest.json` (`DEFAULT_OUT`) | Healthcheck output path | `cli\src\healthcheck.rs:79` |
| `CALYX_SECRET_ENV` | path | `/run/leapable/secrets/calyx.env` (`DEFAULT_SECRET_ENV`) | Rendered secret env file probed by healthcheck | `cli\src\healthcheck.rs:80-81` |
| (healthcheck `CALYX_HOME`) | path | `/home/croyse/calyx` (`DEFAULT_CALYX_HOME`) | Calyx home for healthcheck | `cli\src\healthcheck.rs:82` |
| `CALYX_HEALTH_VAULT` | path | none (optional) | Optional vault path probed by healthcheck | `cli\src\healthcheck.rs:87` |
| `CALYX_HEALTH_METRICS_URL` | string | none (optional) | Optional metrics URL probed by healthcheck | `cli\src\healthcheck.rs:88` |
| `CALYX_PANEL_VRAM_SOFT_CAP_BYTES` | `u64` | `32 GiB` | Parse error → CLI usage error | `cli\src\lens_commands.rs:356` |
| `CALYX_TEI_RESERVED_BYTES` | `u64` | `20 GiB` | Parse error → CLI usage error | `cli\src\lens_commands.rs:357` |
| `CALYX_PANEL_RAM_SOFT_CAP_BYTES` | `u64` | `121 GiB` | Parse error → CLI usage error | `cli\src\lens_commands.rs:359` |
| `CALYX_CPU_LENS_POOL_CAP` | `usize` | `128` | Parse error → CLI usage error | `cli\src\lens_commands.rs:361` |
| `PATH` | OS path list | — | Used by `command_exists` to resolve external commands | `registry\src\spec.rs:195` |

Caller-specific `CALYX_HOME` fallbacks when unset:
- `tiering.rs` / `registry_hot_add.rs`: fall back to `/home/croyse/calyx` (or temp dir for anneal artifacts).
- `ops.rs::tier_roots`: fall back to vault parent, else `.`.
- `lens_commands.rs` / `panel_commands.rs::catalog_path`: **error** ("CALYX_HOME is required or pass --home").
- `config.rs`: leaves `$CALYX_HOME` literal in the path.

### Build-time environment (`crates\calyx-forge\build.rs`)

| Variable | Purpose | Line |
|---|---|---|
| `CARGO_MANIFEST_DIR` | crate manifest dir (standard Cargo) | `build.rs:42` |
| `OUT_DIR` | build output dir (standard Cargo) | `build.rs:43` |
| `CARGO_FEATURE_CUDA` | detects whether the `cuda` feature is enabled | `build.rs:68` |
| `CUDA_PATH` | locates the CUDA toolkit | `build.rs:72` |

`env!("CARGO_PKG_VERSION")` / `env!("CARGO_PKG_NAME")` are compiled into
`calyx-mcp` (`server.rs:83`, `lib.rs:26`) — standard Cargo build-time macros.

---

## 5. Config validation rules

### 5.1 `CalyxConfig` (fail-closed, `crates\calyxd\src\config.rs`)

- **`deny_unknown_fields`** (`config.rs:48`): any unknown/typo'd key →
  `CALYX_DAEMON_CONFIG_INVALID` (test `unknown_key_rejected_fail_closed`).
- **Missing required key** (`vault_path`, `vram_budget_mib`, `log_dir`) → a
  descriptive `CALYX_DAEMON_CONFIG_INVALID` naming the key.
- **TOML syntax error** → `CALYX_DAEMON_CONFIG_INVALID` ("parse calyx config: …").
- **Non-UTF-8 file** → `CALYX_DAEMON_CONFIG_INVALID`.
- **`bind_addr` not loopback** → `CALYX_DAEMON_BIND_FAILED` (accepts `127.0.0.1`
  and `[::1]`; rejects `0.0.0.0`, `[::]`).
- **`vram_budget_mib == 0` or `> 30000`** → `CALYX_FORGE_VRAM_BUDGET`. Boundary:
  `30000` accepted, `30001` rejected (test `ceiling_vram_budget_accepted_one_over_rejected`).
- Validation runs inside `from_toml_str` / `from_file` **before** the struct is
  returned, so a `CalyxConfig` value always upholds its invariants.

### 5.2 Capability gate thresholds (`registry\src\profile\gating.rs`)

`CapabilityGateThresholds::from_env` validates after reading env:
- `min_signal_bits` must be finite and `>= 0.0` → else `assay_low_signal`.
- `max_pairwise_corr` must be finite and within `[0.0, 1.0]` → else `assay_redundant`.

### 5.3 Other env parses (fail-closed)

- `CALYX_FORGE_VRAM_BUDGET`: non-integer → `CALYX_FORGE_VRAM_BUDGET` error.
- `CALYX_ANNEAL_VRAM_BUDGET`: invalid → cap forced to **0** (fail closed), error logged.
- `CALYX_GC_MAX_OPS_PER_RUN` / `CALYX_GC_MIN_INTERVAL_MS`: parse error → error result.
- CLI `env_u64`/`env_usize`/`env_f32` helpers: parse error → CLI usage / assay error;
  `NotPresent` → documented default.

### 5.4 Secondary TOML configs

`BudgetConfig`, `JWeights`, `GoodhartState`, `TripwireFile`, and the navigation spec
structs all use `#[serde(deny_unknown_fields)]`; `BudgetConfig` and `GoodhartState`
additionally run a `validate()` / `validate_state` step after deserialization.

---

## 6. Load / merge precedence

### 6.1 Daemon config (`calyxd`)

`CalyxConfig` is parsed from **one TOML file** (passed via `--config`, e.g.
`infra/aiwonder/calyx.toml`). There is no multi-file merge. Within that file:

1. Keys present in the TOML are used verbatim.
2. Omitted **optional** keys fall back to their serde `default` functions
   (`bind_addr`, `health_log_path`, `tei_endpoints`, `healthcheck_timeout_secs`).
3. Omitted **required** keys (`vault_path`, `vram_budget_mib`, `log_dir`) are a
   hard error — no silent default.
4. After parsing, `vault_path` may interpolate the `CALYX_HOME` **environment
   variable** at resolution time (`vault_path_resolved()`).

So precedence for the daemon is: **TOML value > serde default**, with the
**environment** (`CALYX_HOME`) participating only in `vault_path` interpolation.
Secrets never enter the file; they arrive via environment variables / a rendered
`calyx.env` (documented in `config.rs` and `calyx.toml` headers).

### 6.2 Env-driven tunables (Forge / GC / capability gate / CLI)

For the standalone `from_env` / `env_*` readers: **environment variable value (if
set and valid) > documented constant default**. Invalid values fail closed (error,
or — for `CALYX_ANNEAL_VRAM_BUDGET` — a zero cap), never a silent fallback to the
default.

### 6.3 Cargo target dir (build)

Per `.cargo\config.toml` comments and the `.env.example` `CALYX_CARGO_TARGET_DIR`,
the target directory is supplied on aiwonder per command, not through committed
Cargo config. Verified builds use `scripts/build-verified-calyx.sh`, which exports
`CARGO_TARGET_DIR` for that invocation and reads Cargo metadata back to prove the
resolved target directory. Sourced `env.sh` exposes `CALYX_CARGO_TARGET_DIR` and
clears any inherited `CARGO_TARGET_DIR` outside `CALYX_HOME`; callers that want
the shared target export `CARGO_TARGET_DIR="$CALYX_CARGO_TARGET_DIR"` explicitly.
Cargo resolves precedence as **CLI > env > config file**. The committed
`.cargo\config.toml` deliberately omits `build.target-dir` (an absolute Unix path
would misresolve on Windows).

---

## 7. Toolchain and Cargo configuration

### 7.1 `rust-toolchain.toml`

```toml
[toolchain]
channel = "1.95.0"
profile = "minimal"
components = ["clippy", "rustfmt"]
```

Pins the workspace to Rust **1.95.0**, minimal profile, with `clippy` and `rustfmt`.

### 7.2 `.cargo/config.toml`

```toml
[term]
color = "auto"
```

Only sets terminal color to `auto`. The leading comment block explicitly warns that
**machine-specific settings (e.g. `build.target-dir`) must NOT be committed here**,
because an absolute Unix path resolves incorrectly on Windows. On aiwonder the
shared target dir is exposed as `CALYX_CARGO_TARGET_DIR` by `env.sh`; verified
builds set `CARGO_TARGET_DIR` for the command and read the resolved metadata back
(precedence CLI > env > config).
