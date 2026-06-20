# 03 — Configuration

**Source files covered:**

- `crates/calyxd/src/config.rs` — `CalyxConfig` struct, TOML loading, validation
- `crates/calyxd/src/error.rs` — `DaemonError` taxonomy / `CALYX_*` codes
- `crates/calyxd/src/main.rs` — config-file CLI argument handling, default path
- `crates/calyxd/src/cuda_probe.rs` — `CALYX_FORCE_CUDA_FAIL` env var
- `crates/calyx-forge/src/vram/budget.rs` — `CALYX_FORGE_VRAM_BUDGET` env var
- `crates/calyx-forge/src/vram/yield_policy.rs` — `CALYX_ANNEAL_VRAM_BUDGET` env var
- `crates/calyx-aster/src/gc/snapshot_gc/reclaimer.rs` — `CALYX_GC_*` env vars
- `crates/calyx-registry/src/profile/gating.rs` — `CALYX_CAPABILITY_*` env vars
- `crates/calyx-registry/src/runtime/adapters/lens.rs` — `CALYX_ALLOW_NONCOMMERCIAL_LENSES`
- `crates/calyx-mcp/src/tools/vault/store.rs`, `crates/calyx-cli/src/cmd/vault.rs`, `crates/calyx-anneal/src/janitor.rs`, `crates/calyx-anneal/src/propose/registry_hot_add.rs`, `crates/calyx-aster/src/compaction/tiering.rs` — `CALYX_HOME` consumers
- `crates/calyx-cli/src/healthcheck.rs` — deploy-health env vars and defaults
- `crates/calyx-anneal/src/tripwire.rs` — `.anneal/tripwire.toml`
- `crates/calyx-anneal/src/budget.rs` — `.anneal/budget.toml`
- `.env.example`, `.cargo/config.toml`, `rust-toolchain.toml` (repo root)

See [20_cli_and_daemon_reference.md](20_cli_and_daemon_reference.md) for the CLI flags that
carry these config sources at invocation.

---

## 1. Configuration model overview

Calyx has **no single global config file loaded by every crate.** Configuration arrives
through four distinct mechanisms, in this order of authority for the daemon:

1. **The daemon TOML config** (`CalyxConfig`) — the one authoritative runtime config for
   `calyxd`; passed explicitly via `--config <path>` (§2).
2. **Environment variables** read at runtime by individual subsystems (§3). Secrets enter
   *only* through env vars / a rendered `calyx.env`; they never appear in `CalyxConfig`
   (`crates/calyxd/src/config.rs` module doc).
3. **Per-vault persisted Anneal config files** — `.anneal/tripwire.toml` and
   `.anneal/budget.toml`, auto-created with defaults on first load (§4).
4. **Build/toolchain config** — `.cargo/config.toml`, `rust-toolchain.toml` (§5).

There is **no defaults-vs-file-vs-env merge** inside `CalyxConfig`: per-key TOML `#[serde(default …)]`
functions supply omitted *optional* keys; required keys have no default and a missing one is a
hard error. Env vars are read independently by other crates and are *not* merged into
`CalyxConfig` — the only env interpolation that touches the config is `$CALYX_HOME` expansion
inside `vault_path` (§2.3).

---

## 2. Daemon config: `CalyxConfig`

`crates/calyxd/src/config.rs`, struct `CalyxConfig`.

### 2.1 Format, location, loading

- **Format:** TOML, with `#[serde(deny_unknown_fields)]` — any unknown/typo'd key is a
  fail-closed error, not silently ignored.
- **Default path:** none is auto-discovered at boot. The path is supplied with
  `calyxd --config <path>`. When `--validate-config` or `--audit-vram` is given without
  `--config`, the path defaults to **`calyx.toml`** (relative; `crates/calyxd/src/main.rs`,
  `parse_args`). The module doc and tests reference **`infra/aiwonder/calyx.toml`** as the
  canonical deployed file.
- **Loaders:** `CalyxConfig::from_file(path)` reads bytes → checks UTF-8 → `from_toml_str` →
  `validate`. `CalyxConfig::from_toml_str(text)` parses then validates. There is **no other
  constructor**; every `CalyxConfig` instance is validated.
- A read error, non-UTF-8 file, TOML parse error, or missing required key all yield
  `CALYX_DAEMON_CONFIG_INVALID` (see §6).

### 2.2 Keys

| Key | Type | Required | Default | Valid range / rule | Description |
|---|---|---|---|---|---|
| `bind_addr` | `SocketAddr` (string `ip:port`) | Optional | `127.0.0.1:7700` (`default_bind_addr`) | IP must be **loopback** (`127.0.0.1` or `[::1]`); non-loopback rejected | Address the daemon listens on. |
| `vault_path` | path string | **Required** | none | may contain `$CALYX_HOME` / `${CALYX_HOME}` (see §2.3) | Aster vault directory. |
| `vram_budget_mib` | `u32` | **Required** | none | `1..=30000` (`VRAM_BUDGET_MIB_CEILING = 30_000`) | VRAM budget for Forge, in MiB. Ceiling leaves headroom below the RTX 5090's 32 607 MiB for resident TEI + CUDA context. |
| `log_dir` | path string | **Required** | none | — | Directory for daemon logs. |
| `health_log_path` | path string | Optional | `/zfs/hot/logs/calyx-health/latest.json` (`default_health_log_path`) | — | Path the healthcheck JSON is written to. |
| `tei_endpoints` | array of strings | Optional | `[]` (empty; `#[serde(default)]`) | — | Text-Embeddings-Inference endpoints (documents `:8088`/`:8089`/`:8090`). Documentation only — not validated. |
| `healthcheck_timeout_secs` | `u32` | Optional | `30` (`default_healthcheck_timeout_secs`) | — | Healthcheck timeout in seconds. |

Minimal valid config = the three required keys (`vault_path`, `vram_budget_mib`, `log_dir`);
all optional keys then take the documented defaults (confirmed by test
`parses_minimal_valid_config_and_round_trips_fields`).

Example (from the in-crate `VALID_TOML` test fixture):

```toml
bind_addr = "127.0.0.1:7700"
vault_path = "/zfs/hot/calyx/vault"
vram_budget_mib = 8192
log_dir = "/zfs/hot/logs/calyx"
health_log_path = "/zfs/hot/logs/calyx-health/latest.json"
tei_endpoints = ["http://127.0.0.1:8088", "http://127.0.0.1:8089"]
healthcheck_timeout_secs = 30
```

### 2.3 `$CALYX_HOME` interpolation

`CalyxConfig::vault_path_resolved()` substitutes the `CALYX_HOME` env var (constant
`VAULT_PATH_HOME_VAR = "CALYX_HOME"`) into `vault_path`, replacing both `${CALYX_HOME}` and
`$CALYX_HOME`. If `CALYX_HOME` is unset, the raw literal path is returned unchanged (no silent
expansion to empty). The raw, unexpanded `vault_path` is what is stored and compared; expansion
happens only on demand. The pure helper `resolve_home(path, home)` is separated for testability.

### 2.4 Validation rules

`CalyxConfig::validate(self)` (run by both constructors) enforces, in order:

1. `bind_addr.ip().is_loopback()` must be true → else `CALYX_DAEMON_BIND_FAILED`. `[::1]` is
   accepted; `0.0.0.0` and `[::]` (unspecified) are rejected.
2. `vram_budget_mib != 0 && vram_budget_mib <= 30000` → else `CALYX_FORGE_VRAM_BUDGET`.
   `30000` is accepted; `0` and `30001` are rejected.

TOML-level failures (syntax error, missing required key, unknown key) are caught during parse
and surface as `CALYX_DAEMON_CONFIG_INVALID`.

---

## 3. Environment variables (runtime)

All env vars below are read by non-test code paths. Test-only vars (the many `CALYX_*_FSV_ROOT`
/ `CALYX_ISSUE…_ROOT` / `CALYX_FSV_*` / `CALYX_DETERMINISM` / `PH59_FINAL_SOAK_ROOT` /
`CALYX_PH59_RESTIC_DR` / `CALYX_WAL_RECOVERY_LOCK_FSV_ROOT`) are used only to point FSV fixtures
at an evidence directory and are **not** product configuration; they are excluded here.

### 3.1 Runtime env var table

| Variable | Read in (file) | Purpose | Default if unset |
|---|---|---|---|
| `CALYX_HOME` | `calyxd/src/config.rs` (`vault_path_resolved`); `calyx-mcp/src/tools/vault/store.rs` (`home_dir`); `calyx-cli/src/cmd/vault.rs` (`home_dir`); `calyx-anneal/src/janitor.rs` (`Janitor::new`); `calyx-anneal/src/propose/registry_hot_add.rs` (`default_artifact_dir`); `calyx-aster/src/compaction/tiering.rs`; `calyx-cli/src/healthcheck.rs` (`calyx_home`) | Self-contained Calyx root; used for `$CALYX_HOME` expansion, vault/home resolution, anneal artifact + janitor home, tiering staging parent | Varies: config expansion leaves path literal; `janitor` → `"."`; `healthcheck` → `/home/croyse/calyx` |
| `CALYX_FORGE_VRAM_BUDGET` | `calyx-forge/src/vram/budget.rs` (`VramBudgeter::from_env`) | Forge soft-cap on cumulative VRAM, in **bytes (decimal)** | `DEFAULT_SOFT_CAP_BYTES` = 12 GiB (`12 * 1024^3`). Invalid value → parse error |
| `CALYX_ANNEAL_VRAM_BUDGET` | `calyx-forge/src/vram/yield_policy.rs` (`YieldPolicy::from_env`) | Anneal background-lane VRAM cap, in **bytes** | `DEFAULT_ANNEAL_VRAM_CAP_BYTES` = 2 GiB. Invalid value → **fails closed to cap = 0** (logs the parse error) |
| `CALYX_FORCE_CUDA_FAIL` | `calyxd/src/cuda_probe.rs` (`probe_cuda_device`, const `FORCE_FAIL_ENV`) | Deterministic FSV fault injection: value `"1"` forces `CALYX_FORGE_DEVICE_UNAVAILABLE` | Unset/other ⇒ real CUDA probe runs |
| `CALYX_GC_MAX_OPS_PER_RUN` | `calyx-aster/src/gc/snapshot_gc/reclaimer.rs` (`GcRateLimit::from_env`) | Max snapshot-GC ops per run (`usize`) | `DEFAULT_GC_MAX_OPS_PER_RUN` = 1000. Unparseable → `CALYX_GC_ERROR` |
| `CALYX_GC_MIN_INTERVAL_MS` | `calyx-aster/src/gc/snapshot_gc/reclaimer.rs` (`GcRateLimit::from_env`) | Min ms between GC ticks (`u64`) | `DEFAULT_GC_MIN_INTERVAL_MS` = 1000. Unparseable → `CALYX_GC_ERROR` |
| `CALYX_CAPABILITY_MIN_SIGNAL_BITS` | `calyx-registry/src/profile/gating.rs` (`CapabilityGateThresholds::from_env`) | Min signal bits a lens must carry (`f32`) | `MIN_SIGNAL_BITS` = 0.05. Must be finite & ≥ 0 else `CALYX_ASSAY_LOW_SIGNAL` |
| `CALYX_CAPABILITY_MAX_PAIRWISE_CORR` | `calyx-registry/src/profile/gating.rs` (`CapabilityGateThresholds::from_env`) | Max pairwise correlation allowed (`f32`) | `MAX_PAIRWISE_CORR` = 0.6. Must be finite in `[0,1]` else `CALYX_ASSAY_REDUNDANT` |
| `CALYX_ALLOW_NONCOMMERCIAL_LENSES` | `calyx-registry/src/runtime/adapters/lens.rs` (`allow_noncommercial_from_env`) | Opt-in to register non-commercial-licensed lenses; truthy = `1`/`true`/`yes`/`on` (case-insensitive, trimmed) | Unset ⇒ false; non-commercial lens → `CALYX_LICENSE_DENIED` |
| `CALYX_HEALTH_LOG_PATH` | `calyx-cli/src/healthcheck.rs` (`HealthArgs::parse`, `env_path`) | Deploy-health output JSON path | `/zfs/hot/logs/calyx-health/latest.json` (`DEFAULT_OUT`); overridable by `--out` |
| `CALYX_SECRET_ENV` | `calyx-cli/src/healthcheck.rs` (`env_path`) | Path to rendered secret env probed by deploy-health | `/run/leapable/secrets/calyx.env` (`DEFAULT_SECRET_ENV`); overridable by `--secret-env` |
| `CALYX_HEALTH_METRICS_URL` | `calyx-cli/src/healthcheck.rs` | Prometheus metrics URL to probe | `None` (check skipped); overridable by `--metrics-url` |
| `PATH` | `calyx-registry/src/spec.rs` (`LensSpec` resolution) | Standard OS PATH lookup for lens runtime executables | OS-provided |

Notes:
- `EXA_API_KEY` was searched for across the workspace and is **not** read anywhere in
  `C:\code\Calyx-Dev`. (It belongs to the user's separate Exa MCP setup, not Calyx.)
- The deploy-health command's required-secret check defaults to requiring `HF_HUB_TOKEN` and
  `HF_TOKEN` to be present in the rendered secret env (`DEFAULT_REQUIRED_ENV` in
  `calyx-cli/src/healthcheck.rs`); additional names can be added with `--require-env`.

### 3.2 `.env.example` (repo root)

Tracked template; **names only, no secret values** (Doctrine §8c). Copy to `.env` (gitignored)
and fill in. These variables are deployment/SSH/service plumbing consumed by infra scripts; most
are **not** read by the Rust code (only `CALYX_HOME` from this list is consumed at runtime — §3.1).

| Variable | Example value in template | Purpose |
|---|---|---|
| `AIWONDER_VPN_SERVER` | (blank) | Cisco AnyConnect VPN server |
| `AIWONDER_VPN_USER` | (blank) | VPN user |
| `AIWONDER_VPN_PASSWORD` | (blank) | VPN password (secret) |
| `AIWONDER_HOST` | `aiwonder.mst.com` | SSH host |
| `AIWONDER_IP` | (blank) | SSH IP |
| `AIWONDER_SSH_PORT` | `22` | SSH port |
| `AIWONDER_USER` | `croyse` | SSH user |
| `AIWONDER_SSH_PASSWORD` | (blank) | SSH password (secret) |
| `AIWONDER_SUDO_PASSWORD` | (blank) | sudo password (secret) |
| `CALYX_HOME` | `/home/croyse/calyx` | Self-contained Calyx root (also runtime — §3.1) |
| `CALYX_HOT_DATASET` | `hotpool/calyx` | ZFS hot dataset name |
| `CALYX_COLD_DATASET` | `archive/calyx` | ZFS cold dataset name |
| `CALYX_HOT_PATH` | `/zfs/hot/calyx` | Hot tier mount path |
| `CALYX_COLD_PATH` | `/zfs/archive/calyx` | Cold tier mount path |
| `CALYX_CARGO_TARGET_DIR` | `/home/croyse/calyx/target` | Shared cargo target dir (used by env.sh, not `.cargo/config.toml`) |
| `CALYX_RUST_ENV` | `/home/croyse/.cargo/env` | Path to cargo env script |
| `AIWONDER_TEI_GENERAL` | `http://127.0.0.1:8088` | Resident TEI general embedder |
| `AIWONDER_TEI_RERANKER` | `http://127.0.0.1:8089` | Resident TEI reranker |
| `AIWONDER_TEI_LEGAL` | `http://127.0.0.1:8090` | Resident TEI legal embedder |
| `AIWONDER_PROMETHEUS` | `http://127.0.0.1:9090` | Prometheus endpoint |
| `HF_TOKEN` | (blank) | Hugging Face token (secret) |
| `HF_HUB_TOKEN` | (blank) | Hugging Face Hub token (secret) |
| `HF_HOME` | `/home/croyse/calyx/.hf-cache` | HF cache dir |
| `CALYX_REPO` | `chrisroyse/calyx-dev` | GitHub dev-state repo slug |

---

## 4. Anneal per-vault config files

The Anneal crate persists two TOML files under `<vault>/.anneal/`. Both are **created with
defaults on first load if absent**, and read back (round-tripped) thereafter. The directory
constant is `.anneal` (`CONFIG_DIR`) in both modules.

### 4.1 `.anneal/tripwire.toml`

`crates/calyx-anneal/src/tripwire.rs`. Path = `<vault>/.anneal/tripwire.toml`
(`tripwire_config_path`). Loaded by `TripwireRegistry::load_from_vault`.

On-disk shape: a `[thresholds]` table keyed by metric name, each value a
`{ bound, hysteresis, direction }` record. All five metrics must be present (an unknown metric
key, or a missing one, → `CALYX_TRIPWIRE_INVALID_CONFIG`).

Default thresholds (`default_bound` / `default_direction`; default hysteresis =
`bound * DEFAULT_HYSTERESIS_FRACTION`, `DEFAULT_HYSTERESIS_FRACTION = 0.05`):

| TOML metric key | `bound` (default) | `hysteresis` (default = 5% of bound) | `direction` |
|---|---|---|---|
| `recall_at_k` | 0.90 | 0.045 | `below` |
| `guard_far` | 0.01 | 0.0005 | `above` |
| `guard_frr` | 0.05 | 0.0025 | `above` |
| `search_p99` | 200.0 | 10.0 | `above` |
| `ingest_p95` | 500.0 | 25.0 | `above` |

`direction` serializes snake_case (`below` / `above`). A non-finite measured metric value →
`CALYX_TRIPWIRE_INVALID_METRIC`.

### 4.2 `.anneal/budget.toml`

`crates/calyx-anneal/src/budget.rs`. Path = `<vault>/.anneal/budget.toml`
(`budget_config_path`). Loaded by `BudgetConfig::load_from_vault`.

| Key | Type | Default | Valid range | Description |
|---|---|---|---|---|
| `cpu_fraction` | `f64` | `0.15` (`DEFAULT_CPU_FRACTION`) | finite, `0.0..=1.0` | Fraction of CPU the background lane may reserve |
| `vram_bytes` | `u64` | `536870912` (512 MiB, `DEFAULT_VRAM_BYTES`) | — | VRAM bytes budgeted to the background lane |
| `tick_interval_ms` | `u64` | `100` (`DEFAULT_TICK_INTERVAL_MS`) | must be > 0 | Cooperative tick interval |

`BudgetConfig::validate` enforces the `cpu_fraction` range and non-zero `tick_interval_ms`;
violations → `CALYX_ANNEAL_BUDGET_INVALID_CONFIG`. Related runtime codes:
`CALYX_ANNEAL_BUDGET_EXHAUSTED`, `CALYX_ANNEAL_BUDGET_NVML_UNAVAILABLE`. Background nice level
is fixed at `BACKGROUND_NICE = 10`.

---

## 5. Build / toolchain config

### 5.1 `rust-toolchain.toml`

```toml
[toolchain]
channel = "1.95.0"
profile = "minimal"
components = ["clippy", "rustfmt"]
```

Pins the workspace to Rust **1.95.0** (matches the spec's "toolchain 1.95"), minimal profile,
with `clippy` and `rustfmt` components.

### 5.2 `.cargo/config.toml`

```toml
[term]
color = "auto"
```

Deliberately **machine-agnostic** — the file's header comment warns that machine-specific
settings (e.g. `build.target-dir`) must NOT be committed here, because an absolute Unix path
silently resolves to `<drive>:\home\...` on Windows. The shared target dir on aiwonder is
provided instead by `env.sh` via the `CARGO_TARGET_DIR` env var, which takes precedence over this
file (precedence: CLI > env > config). Only `[term].color = "auto"` is set.

There is no `.config/` directory with additional runtime config beyond the above.

---

## 6. Validation error codes (the config taxonomy)

`DaemonError` (`crates/calyxd/src/error.rs`) maps to stable `CALYX_*` codes. `Display` always
renders `<code>: <detail> (remediation: <hint>)`.

| Variant | Code | Triggered by (config-relevant) |
|---|---|---|
| `BindFailed` | `CALYX_DAEMON_BIND_FAILED` | non-loopback `bind_addr` |
| `ConfigInvalid` | `CALYX_DAEMON_CONFIG_INVALID` | TOML syntax error, missing required key, **unknown key** (`deny_unknown_fields`), non-UTF-8 file, read error, bad CLI arg |
| `VramBudget` | `CALYX_FORGE_VRAM_BUDGET` | `vram_budget_mib` outside `1..=30000`; also Forge runtime soft-cap breaches |
| `DeviceUnavailable` | `CALYX_FORGE_DEVICE_UNAVAILABLE` | CUDA init failure / `CALYX_FORCE_CUDA_FAIL=1` |
| `HealthFailed` | `CALYX_DAEMON_HEALTH_FAIL` | daemon-readiness probe failure not covered by a more specific code |

The single error code for an invalid daemon config file is **`CALYX_DAEMON_CONFIG_INVALID`**.

Other config-validation codes from sibling crates: `CALYX_TRIPWIRE_INVALID_CONFIG`,
`CALYX_TRIPWIRE_INVALID_METRIC` (§4.1); `CALYX_ANNEAL_BUDGET_INVALID_CONFIG` (§4.2);
`CALYX_GC_ERROR` (bad `CALYX_GC_*` value); `CALYX_ASSAY_LOW_SIGNAL` / `CALYX_ASSAY_REDUNDANT`
(bad `CALYX_CAPABILITY_*` value); `CALYX_LICENSE_DENIED` (non-commercial lens without opt-in);
`CALYX_HEALTH_CONFIG_INVALID` / `CALYX_HEALTHCHECK_FAILED` (deploy-health CLI).

---

## Gaps / not covered

- `tei_endpoints` is documentation-only: it is parsed and stored but **not** validated or
  otherwise acted on inside `CalyxConfig` (no connectivity check at config-load time).
- `CalyxConfig` does **not** read any environment variable except `CALYX_HOME` for `vault_path`
  expansion; env vars in §3 are consumed by other crates independently and are not merged into
  the daemon config struct.
- The canonical deployed config path `infra/aiwonder/calyx.toml` is referenced in code/tests but
  the file itself was not read for this doc (it lives in `infra/`, outside the crate tree);
  field semantics here are derived entirely from the `CalyxConfig` struct.
- Numerous `CALYX_*_FSV_ROOT` / `CALYX_ISSUE*` / `CALYX_FSV_*` env vars exist only in test code
  to locate fail-shut-verification evidence directories; they are intentionally excluded as
  non-product configuration.
