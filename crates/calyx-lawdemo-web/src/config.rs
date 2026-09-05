//! Startup configuration — everything env-driven via `LAWDEMO_WEB_*` vars.
//!
//! NO hard-coded ULIDs, vault paths, binary paths, or SHA-256 pins live in the
//! source: the demo topology is in flux under #1955, so every deployment fact is
//! supplied by the environment (mirroring `tools/lawdemo/run_demo.sh`). Missing
//! required config at startup is a hard panic with a clear message (fail-fast) —
//! the service never boots into a half-configured state.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// Default loopback bind. The server REFUSES to start on a non-loopback address.
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8600";

/// A historical demo artifact served independently at `GET /api/demo/4`.
///
/// The live console never consumes this artifact as substitute state. The file
/// (relative to the evidence dir) and its pinned SHA-256 are retained for explicit
/// evidence inspection; bytes are verified against this pin before ever being
/// served.
#[derive(Debug, Clone)]
pub struct ArtifactSpec {
    /// Demo index (the service exposes `4`, the historical corpus census).
    pub demo: u8,
    /// File name inside [`Config::evidence_dir`].
    pub filename: String,
    /// Lowercase 64-char hex SHA-256 the file bytes must match.
    pub sha256_hex: String,
}

/// Fully-resolved, validated runtime configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Loopback bind address (asserted loopback in [`Config::from_map`]).
    pub bind_addr: SocketAddr,
    /// Absolute path to the pinned `calyx` binary used for live mode.
    pub calyx_binary: PathBuf,
    /// `CALYX_HOME` for every live subprocess. The pinned binary's vault commands
    /// (`kernel-answer`, `reproduce`, `verify-chain`, `ledger-tail`) REQUIRE it —
    /// without it they fail at pitch time with `CALYX_CLI_USAGE_ERROR: CALYX_HOME
    /// is required for vault commands`. Required at startup and passed EXPLICITLY
    /// into each subprocess env (never merely inherited).
    pub calyx_home: PathBuf,
    /// Optional `ORT_DYLIB_PATH` for the ONNX Runtime dynamic library. When set it
    /// is passed through to every live subprocess. Per #1951, an UNSET
    /// `ORT_DYLIB_PATH` turns a lens-loading failure into an indefinite hang, so a
    /// deployment that relies on lens loading should set it explicitly.
    pub ort_dylib_path: Option<String>,
    /// Path to the audited lens endpoint relocation table (#2053). Passed
    /// explicitly to every spawned `calyx`, because a subprocess that silently
    /// missed it would dial the foreign endpoints the relocation exists to
    /// escape and fail the doctrine kernel exactly as the outage did.
    pub lens_relocation: Option<String>,
    /// `host:port` of the running resident (kernel-answer / readiness).
    pub resident_addr: String,
    /// Production vault path (demo-5 live answer / reproduce / verify-chain).
    pub vault: String,
    /// Demo-1 scoped-vault path (doctrine kernel) — echoed in health.
    pub d1_vault: String,
    /// Demo-1 kernel id — echoed in health.
    pub d1_kernel: String,
    /// Demo-2 scoped-vault path (judge intel) — echoed in health.
    pub d2_vault: String,
    /// Demo-2 kernel id — echoed in health.
    pub d2_kernel: String,
    /// Directory holding the SHA-pinned corpus-census artifact.
    pub evidence_dir: PathBuf,
    /// Human label for the evidence source (e.g. `development-sample`,
    /// `production-fsv`) surfaced in health/provenance so a sample deployment is
    /// never mistaken for the real FSV evidence.
    pub evidence_label: String,
    /// The artifact-mode datum served over `/api/demo/{demo}` — the demo-4 corpus
    /// census the console loads at boot (a one-element set).
    pub artifacts: Vec<ArtifactSpec>,
    /// Hard time budget for every live subprocess. Kernel ranking is served by
    /// the verified long-lived resident, so it receives no inflated cold-load
    /// timeout.
    pub live_timeout: Duration,
    /// Time budget for the resident-readiness probe in `/api/health`.
    pub resident_probe_timeout: Duration,
    /// Research-chat configuration, present iff `LAWDEMO_WEB_CHAT_ENABLED=1`. `None`
    /// means the `/api/chat` route returns the typed `CALYX_LAWDEMO_CHAT_DISABLED`
    /// envelope. The DeepSeek API key it holds is NEVER logged or echoed.
    pub chat: Option<ChatConfig>,
}

/// DeepSeek reasoning-agent configuration for the research-chat surface.
///
/// The `api_key` is redacted in [`std::fmt::Debug`] so a `{:?}` of the parent
/// [`Config`] can never leak it into a log line.
#[derive(Clone)]
pub struct ChatConfig {
    /// DeepSeek API key. NEVER logged, NEVER echoed in `/api/health`.
    pub api_key: String,
    /// OpenAI-compatible base URL (e.g. `https://api.deepseek.com`).
    pub base_url: String,
    /// Supported DeepSeek model id (production uses `deepseek-v4-flash`).
    pub model: String,
    /// Max tool calls per episode (fail-closed budget).
    pub max_tool_calls: u32,
    /// Max reasoning turns per episode.
    pub max_turns: u32,
    /// Per-turn response token cap (keeps reasoning from eating the whole budget).
    pub max_tokens: u32,
    /// Hard wall-clock budget for a whole episode.
    pub wall_timeout: Duration,
    /// Per-HTTP-request timeout for a single DeepSeek call.
    pub request_timeout: Duration,
    /// Upper clamp on `corpus_search`/`cite_check` `k`.
    pub search_k_max: u32,
    /// How long a RUNNING episode may have zero attached clients before the reaper
    /// cancels it as abandoned (#2061).
    ///
    /// This is the bound on orphaned DeepSeek spend and orphaned GPU-search permits.
    /// It must be comfortably longer than a browser reattach round-trip (a frozen tab
    /// keeps its socket, so it still counts as attached; a discarded tab, a closed
    /// tab, or a dropped tunnel does not), and comfortably shorter than the wall
    /// budget, or an abandoned episode simply runs to `wall_timeout` as before.
    pub client_grace: Duration,
    /// How long a TERMINAL episode is retained for replay after it finishes, so a
    /// client that was frozen/discarded across the finish line can still reattach and
    /// collect the answer it paid for.
    pub episode_retention: Duration,
    /// Hard cap on the bytes one episode's replay log may hold. Past it the log stops
    /// growing and records the truncation point; a reattach that lands inside the lost
    /// range is REFUSED with `CALYX_LAWDEMO_EPISODE_REPLAY_TRUNCATED` rather than
    /// served as a silently incomplete episode.
    pub replay_max_bytes: usize,
}

impl std::fmt::Debug for ChatConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatConfig")
            .field("api_key", &"***redacted***")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("max_tool_calls", &self.max_tool_calls)
            .field("max_turns", &self.max_turns)
            .field("max_tokens", &self.max_tokens)
            .field("wall_timeout", &self.wall_timeout)
            .field("request_timeout", &self.request_timeout)
            .field("search_k_max", &self.search_k_max)
            .field("client_grace", &self.client_grace)
            .field("episode_retention", &self.episode_retention)
            .field("replay_max_bytes", &self.replay_max_bytes)
            .finish()
    }
}

impl ChatConfig {
    /// Parse chat config from env. Returns `Ok(None)` unless `LAWDEMO_WEB_CHAT_ENABLED`
    /// is exactly `1`; when enabled, the three DeepSeek vars are REQUIRED (fail-fast).
    pub fn from_map_if_enabled(map: &HashMap<String, String>) -> Result<Option<Self>, String> {
        if optional(map, "LAWDEMO_WEB_CHAT_ENABLED", "0").trim() != "1" {
            return Ok(None);
        }
        let wall_timeout = parse_secs(map, "LAWDEMO_WEB_CHAT_WALL_TIMEOUT_SECS", 300)?;
        let client_grace = parse_secs(map, "LAWDEMO_WEB_CHAT_CLIENT_GRACE_SECS", 90)?;
        // A grace at/above the wall budget can never fire before the episode ends on
        // its own, which would silently reinstate the orphaned-episode class #2061
        // exists to close. Refuse the config instead of booting a defeated reaper.
        if client_grace >= wall_timeout {
            return Err(format!(
                "LAWDEMO_WEB_CHAT_CLIENT_GRACE_SECS ({}s) must be shorter than \
                 LAWDEMO_WEB_CHAT_WALL_TIMEOUT_SECS ({}s), or an abandoned episode can \
                 never be reaped before its own wall budget ends it",
                client_grace.as_secs(),
                wall_timeout.as_secs()
            ));
        }
        // A zero grace would cancel an episode the instant a client blinks, and a
        // sub-second grace cannot survive a reattach round trip over the tunnel.
        if client_grace < Duration::from_secs(5) {
            return Err(format!(
                "LAWDEMO_WEB_CHAT_CLIENT_GRACE_SECS must be at least 5s so a reattach \
                 round-trip can win the race, got {}s",
                client_grace.as_secs()
            ));
        }
        let replay_max_bytes = parse_usize(map, "LAWDEMO_WEB_CHAT_REPLAY_MAX_BYTES", 32 << 20)?;
        // Below one tool payload the log could not hold even a single event, so every
        // reattach would truncate. That is a broken deployment, not a tight budget.
        if replay_max_bytes < (1 << 20) {
            return Err(format!(
                "LAWDEMO_WEB_CHAT_REPLAY_MAX_BYTES must be at least 1048576 (one tool \
                 payload) or no episode is resumable, got {replay_max_bytes}"
            ));
        }
        let model = require(map, "LEGAL_DEEPSEEK_MODEL")?;
        if !matches!(model.as_str(), "deepseek-v4-flash" | "deepseek-v4-pro") {
            return Err(format!(
                "LEGAL_DEEPSEEK_MODEL must be a supported non-deprecated DeepSeek model \
                 (deepseek-v4-flash or deepseek-v4-pro), got {model:?}"
            ));
        }
        Ok(Some(Self {
            api_key: require(map, "DEEPSEEK_API_KEY")?,
            base_url: require(map, "LEGAL_DEEPSEEK_BASE_URL")?,
            model,
            max_tool_calls: parse_u32(map, "LAWDEMO_WEB_CHAT_MAX_TOOL_CALLS", 40)?,
            max_turns: parse_u32(map, "LAWDEMO_WEB_CHAT_MAX_TURNS", 16)?,
            max_tokens: parse_u32(map, "LAWDEMO_WEB_CHAT_MAX_TOKENS", 3000)?,
            // The episode wall spans several model turns and tool calls; it is
            // intentionally broader than any one subprocess budget.
            wall_timeout,
            request_timeout: parse_secs(map, "LAWDEMO_WEB_CHAT_REQUEST_TIMEOUT_SECS", 120)?,
            search_k_max: parse_u32(map, "LAWDEMO_WEB_CHAT_SEARCH_K_MAX", 20)?,
            client_grace,
            episode_retention: parse_secs(map, "LAWDEMO_WEB_CHAT_EPISODE_RETENTION_SECS", 900)?,
            replay_max_bytes,
        }))
    }
}

fn require(map: &HashMap<String, String>, key: &str) -> Result<String, String> {
    match map.get(key) {
        Some(v) if !v.trim().is_empty() => Ok(v.trim().to_owned()),
        _ => Err(format!(
            "missing required environment variable {key} (set it before starting calyx-lawdemo-web)"
        )),
    }
}

fn optional(map: &HashMap<String, String>, key: &str, default: &str) -> String {
    map.get(key)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(default)
        .to_owned()
}

/// An optional string var: `Some(trimmed)` when present and non-empty, else `None`.
fn optional_opt(map: &HashMap<String, String>, key: &str) -> Option<String> {
    map.get(key)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn parse_secs(map: &HashMap<String, String>, key: &str, default: u64) -> Result<Duration, String> {
    match map.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
        None => Ok(Duration::from_secs(default)),
        Some(raw) => raw
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|_| format!("{key} must be a whole number of seconds, got {raw:?}")),
    }
}

fn parse_u32(map: &HashMap<String, String>, key: &str, default: u32) -> Result<u32, String> {
    match map.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
        None => Ok(default),
        Some(raw) => raw
            .parse::<u32>()
            .map_err(|_| format!("{key} must be a whole non-negative number, got {raw:?}")),
    }
}

fn parse_usize(map: &HashMap<String, String>, key: &str, default: usize) -> Result<usize, String> {
    match map.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
        None => Ok(default),
        Some(raw) => raw.parse::<usize>().map_err(|_| {
            format!("{key} must be a whole non-negative number of bytes, got {raw:?}")
        }),
    }
}

fn validate_sha256(key: &str, value: &str) -> Result<String, String> {
    let v = value.trim().to_ascii_lowercase();
    if v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(v)
    } else {
        Err(format!(
            "{key} must be a 64-character hex SHA-256 digest, got {value:?}"
        ))
    }
}

impl Config {
    /// Build from the process environment (the binary's entrypoint). Panics via
    /// the caller on any missing/invalid required var.
    pub fn from_env() -> Result<Self, String> {
        let map: HashMap<String, String> = std::env::vars().collect();
        Self::from_map(&map)
    }

    /// Build from an explicit key→value map (testable, no process-env mutation).
    pub fn from_map(map: &HashMap<String, String>) -> Result<Self, String> {
        let bind_raw = optional(map, "LAWDEMO_WEB_BIND_ADDR", DEFAULT_BIND_ADDR);
        let bind_addr: SocketAddr = bind_raw.parse().map_err(|e| {
            format!("LAWDEMO_WEB_BIND_ADDR {bind_raw:?} is not a socket address: {e}")
        })?;
        if !bind_addr.ip().is_loopback() {
            return Err(format!(
                "LAWDEMO_WEB_BIND_ADDR must be loopback (127.0.0.1/::1); refusing {bind_addr}"
            ));
        }

        // The service exposes exactly one historical artifact: the demo-4 census.
        let mut artifacts = Vec::with_capacity(1);
        let defaults = [(4u8, "demo4_dissent.json")];
        for (demo, default_file) in defaults {
            let filename = optional(map, &format!("LAWDEMO_WEB_DEMO{demo}_FILE"), default_file);
            let sha_key = format!("LAWDEMO_WEB_DEMO{demo}_SHA");
            let sha256_hex = validate_sha256(&sha_key, &require(map, &sha_key)?)?;
            artifacts.push(ArtifactSpec {
                demo,
                filename,
                sha256_hex,
            });
        }

        Ok(Self {
            bind_addr,
            calyx_binary: PathBuf::from(require(map, "LAWDEMO_WEB_CALYX_BINARY")?),
            calyx_home: PathBuf::from(require(map, "LAWDEMO_WEB_CALYX_HOME")?),
            ort_dylib_path: optional_opt(map, "LAWDEMO_WEB_ORT_DYLIB_PATH"),
            lens_relocation: optional_opt(map, "LAWDEMO_WEB_LENS_RELOCATION"),
            resident_addr: require(map, "LAWDEMO_WEB_RESIDENT_ADDR")?,
            vault: require(map, "LAWDEMO_WEB_VAULT")?,
            d1_vault: require(map, "LAWDEMO_WEB_D1_VAULT")?,
            d1_kernel: require(map, "LAWDEMO_WEB_D1_KERNEL")?,
            d2_vault: require(map, "LAWDEMO_WEB_D2_VAULT")?,
            d2_kernel: require(map, "LAWDEMO_WEB_D2_KERNEL")?,
            evidence_dir: PathBuf::from(require(map, "LAWDEMO_WEB_EVIDENCE_DIR")?),
            evidence_label: optional(map, "LAWDEMO_WEB_EVIDENCE_LABEL", "unlabeled"),
            artifacts,
            live_timeout: parse_secs(map, "LAWDEMO_WEB_LIVE_TIMEOUT_SECS", 120)?,
            resident_probe_timeout: parse_secs(map, "LAWDEMO_WEB_RESIDENT_PROBE_TIMEOUT_SECS", 4)?,
            chat: ChatConfig::from_map_if_enabled(map)?,
        })
    }

    /// Look up the artifact spec for a demo index, if any.
    pub fn artifact(&self, demo: u8) -> Option<&ArtifactSpec> {
        self.artifacts.iter().find(|a| a.demo == demo)
    }

    /// Apply the environment EVERY spawned `calyx` subprocess needs: `CALYX_HOME`
    /// (always, so vault commands never fail with the usage error) and
    /// `ORT_DYLIB_PATH` (only when configured). Both are set explicitly rather
    /// than relying on the service's inherited environment, so a subprocess is
    /// never at the mercy of how the service was launched.
    pub fn apply_subprocess_env(&self, cmd: &mut tokio::process::Command) {
        cmd.env("CALYX_HOME", &self.calyx_home);
        if let Some(ort) = &self.ort_dylib_path {
            cmd.env("ORT_DYLIB_PATH", ort);
        }
        if let Some(relocation) = &self.lens_relocation {
            cmd.env("CALYX_LENS_ENDPOINT_RELOCATION", relocation);
        }
    }
}

/// The minimal set of required env vars for a valid config, for tests across modules.
#[cfg(test)]
pub(crate) fn base_env_map() -> HashMap<String, String> {
    [
        ("LAWDEMO_WEB_CALYX_BINARY", "/opt/calyx/runtime/calyx"),
        ("LAWDEMO_WEB_CALYX_HOME", "/var/lib/calyx"),
        ("LAWDEMO_WEB_RESIDENT_ADDR", "127.0.0.1:18401"),
        ("LAWDEMO_WEB_VAULT", "/zfs/hot/calyx/vaults/PROD"),
        ("LAWDEMO_WEB_D1_VAULT", "/zfs/hot/calyx/vaults/D1"),
        ("LAWDEMO_WEB_D1_KERNEL", "b8f96b5deea45b743229242283234f9e"),
        ("LAWDEMO_WEB_D2_VAULT", "/zfs/hot/calyx/vaults/D2"),
        ("LAWDEMO_WEB_D2_KERNEL", "b38822aad594ab8c521b64683c118f36"),
        ("LAWDEMO_WEB_EVIDENCE_DIR", "/opt/evidence"),
        ("LAWDEMO_WEB_DEMO4_SHA", &"d".repeat(64)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_map() -> HashMap<String, String> {
        let pairs = [
            ("LAWDEMO_WEB_CALYX_BINARY", "/opt/calyx/runtime/calyx"),
            ("LAWDEMO_WEB_CALYX_HOME", "/var/lib/calyx"),
            ("LAWDEMO_WEB_RESIDENT_ADDR", "127.0.0.1:18401"),
            ("LAWDEMO_WEB_VAULT", "/zfs/hot/calyx/vaults/PROD"),
            ("LAWDEMO_WEB_D1_VAULT", "/zfs/hot/calyx/vaults/D1"),
            ("LAWDEMO_WEB_D1_KERNEL", "b8f96b5deea45b743229242283234f9e"),
            ("LAWDEMO_WEB_D2_VAULT", "/zfs/hot/calyx/vaults/D2"),
            ("LAWDEMO_WEB_D2_KERNEL", "b38822aad594ab8c521b64683c118f36"),
            ("LAWDEMO_WEB_EVIDENCE_DIR", "/opt/evidence"),
            ("LAWDEMO_WEB_DEMO4_SHA", &"d".repeat(64)),
        ];
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    #[test]
    fn full_config_parses_with_defaults() {
        let cfg = Config::from_map(&full_map()).expect("valid config");
        assert_eq!(cfg.bind_addr.to_string(), "127.0.0.1:8600");
        assert!(cfg.bind_addr.ip().is_loopback());
        // The service exposes exactly one historical artifact: the demo-4 census.
        assert_eq!(cfg.artifacts.len(), 1);
        assert_eq!(cfg.artifact(4).unwrap().filename, "demo4_dissent.json");
        assert!(cfg.artifact(1).is_none());
        assert_eq!(cfg.live_timeout, Duration::from_secs(120));
        assert_eq!(cfg.evidence_label, "unlabeled");
        assert_eq!(cfg.calyx_home, PathBuf::from("/var/lib/calyx"));
        assert!(cfg.ort_dylib_path.is_none());
    }

    #[test]
    fn calyx_home_is_required() {
        let mut map = full_map();
        map.remove("LAWDEMO_WEB_CALYX_HOME");
        let err = Config::from_map(&map).unwrap_err();
        assert!(err.contains("LAWDEMO_WEB_CALYX_HOME"), "got: {err}");
        assert!(err.contains("missing required"), "got: {err}");
    }

    #[test]
    fn empty_calyx_home_is_treated_as_missing() {
        let mut map = full_map();
        map.insert("LAWDEMO_WEB_CALYX_HOME".into(), "   ".into());
        let err = Config::from_map(&map).unwrap_err();
        assert!(err.contains("LAWDEMO_WEB_CALYX_HOME"), "got: {err}");
    }

    #[test]
    fn ort_dylib_path_is_optional_and_captured_when_set() {
        let mut map = full_map();
        map.insert(
            "LAWDEMO_WEB_ORT_DYLIB_PATH".into(),
            "/opt/onnxruntime/lib/libonnxruntime.so".into(),
        );
        let cfg = Config::from_map(&map).expect("valid config");
        assert_eq!(
            cfg.ort_dylib_path.as_deref(),
            Some("/opt/onnxruntime/lib/libonnxruntime.so")
        );
    }

    #[test]
    fn subprocess_env_sets_calyx_home_always_and_ort_only_when_set() {
        // Without ORT: only CALYX_HOME is applied.
        let cfg = Config::from_map(&full_map()).unwrap();
        let mut cmd = tokio::process::Command::new("/bin/true");
        cfg.apply_subprocess_env(&mut cmd);
        let envs: Vec<(String, Option<String>)> = cmd
            .as_std()
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(envs.contains(&("CALYX_HOME".to_owned(), Some("/var/lib/calyx".to_owned()))));
        assert!(!envs.iter().any(|(k, _)| k == "ORT_DYLIB_PATH"));

        // With ORT: both are applied.
        let mut map = full_map();
        map.insert(
            "LAWDEMO_WEB_ORT_DYLIB_PATH".into(),
            "/opt/onnxruntime/lib/libonnxruntime.so".into(),
        );
        let cfg = Config::from_map(&map).unwrap();
        let mut cmd = tokio::process::Command::new("/bin/true");
        cfg.apply_subprocess_env(&mut cmd);
        let envs: Vec<(String, Option<String>)> = cmd
            .as_std()
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(envs.contains(&(
            "ORT_DYLIB_PATH".to_owned(),
            Some("/opt/onnxruntime/lib/libonnxruntime.so".to_owned())
        )));
    }

    #[test]
    fn missing_required_var_is_a_clear_error() {
        let mut map = full_map();
        map.remove("LAWDEMO_WEB_VAULT");
        let err = Config::from_map(&map).unwrap_err();
        assert!(err.contains("LAWDEMO_WEB_VAULT"), "got: {err}");
        assert!(err.contains("missing required"), "got: {err}");
    }

    #[test]
    fn empty_required_var_is_treated_as_missing() {
        let mut map = full_map();
        map.insert("LAWDEMO_WEB_CALYX_BINARY".into(), "   ".into());
        let err = Config::from_map(&map).unwrap_err();
        assert!(err.contains("LAWDEMO_WEB_CALYX_BINARY"), "got: {err}");
    }

    #[test]
    fn non_loopback_bind_is_refused() {
        let mut map = full_map();
        map.insert("LAWDEMO_WEB_BIND_ADDR".into(), "0.0.0.0:8600".into());
        let err = Config::from_map(&map).unwrap_err();
        assert!(err.contains("loopback"), "got: {err}");
    }

    #[test]
    fn bad_sha_pin_is_rejected() {
        let mut map = full_map();
        map.insert("LAWDEMO_WEB_DEMO4_SHA".into(), "not-a-hash".into());
        let err = Config::from_map(&map).unwrap_err();
        assert!(err.contains("LAWDEMO_WEB_DEMO4_SHA"), "got: {err}");
        assert!(err.contains("hex SHA-256"), "got: {err}");
    }

    #[test]
    fn bad_timeout_is_rejected() {
        let mut map = full_map();
        map.insert("LAWDEMO_WEB_LIVE_TIMEOUT_SECS".into(), "soon".into());
        let err = Config::from_map(&map).unwrap_err();
        assert!(err.contains("LAWDEMO_WEB_LIVE_TIMEOUT_SECS"), "got: {err}");
    }

    #[test]
    fn chat_is_disabled_by_default() {
        let cfg = Config::from_map(&full_map()).unwrap();
        assert!(cfg.chat.is_none());
    }

    #[test]
    fn chat_enabled_requires_the_three_deepseek_vars() {
        let mut map = base_env_map();
        map.insert("LAWDEMO_WEB_CHAT_ENABLED".into(), "1".into());
        // Missing DEEPSEEK_API_KEY → fail-fast.
        let err = Config::from_map(&map).unwrap_err();
        assert!(err.contains("DEEPSEEK_API_KEY"), "got: {err}");
        map.insert("DEEPSEEK_API_KEY".into(), "sk-x".into());
        map.insert(
            "LEGAL_DEEPSEEK_BASE_URL".into(),
            "https://api.deepseek.com".into(),
        );
        map.insert("LEGAL_DEEPSEEK_MODEL".into(), "deepseek-v4-flash".into());
        let cfg = Config::from_map(&map).unwrap();
        let chat = cfg.chat.expect("chat enabled");
        assert_eq!(chat.model, "deepseek-v4-flash");
        assert_eq!(chat.max_tool_calls, 40);
        assert_eq!(chat.max_turns, 16);
        assert_eq!(chat.wall_timeout.as_secs(), 300);
        assert_eq!(chat.search_k_max, 20);
    }

    #[test]
    fn chat_budgets_are_env_tunable() {
        let mut map = base_env_map();
        for (k, v) in [
            ("LAWDEMO_WEB_CHAT_ENABLED", "1"),
            ("DEEPSEEK_API_KEY", "sk-x"),
            ("LEGAL_DEEPSEEK_BASE_URL", "https://api.deepseek.com"),
            ("LEGAL_DEEPSEEK_MODEL", "deepseek-v4-flash"),
            ("LAWDEMO_WEB_CHAT_MAX_TOOL_CALLS", "5"),
            ("LAWDEMO_WEB_CHAT_MAX_TOKENS", "1234"),
            ("LAWDEMO_WEB_CHAT_SEARCH_K_MAX", "7"),
        ] {
            map.insert(k.into(), v.into());
        }
        let chat = Config::from_map(&map).unwrap().chat.unwrap();
        assert_eq!(chat.max_tool_calls, 5);
        assert_eq!(chat.max_tokens, 1234);
        assert_eq!(chat.search_k_max, 7);
    }

    #[test]
    fn chat_debug_redacts_the_api_key() {
        let mut map = base_env_map();
        for (k, v) in [
            ("LAWDEMO_WEB_CHAT_ENABLED", "1"),
            ("DEEPSEEK_API_KEY", "sk-SUPER-SECRET-VALUE"),
            ("LEGAL_DEEPSEEK_BASE_URL", "https://api.deepseek.com"),
            ("LEGAL_DEEPSEEK_MODEL", "deepseek-v4-flash"),
        ] {
            map.insert(k.into(), v.into());
        }
        let cfg = Config::from_map(&map).unwrap();
        let dbg = format!("{cfg:?}");
        assert!(
            !dbg.contains("sk-SUPER-SECRET-VALUE"),
            "api key leaked in Debug"
        );
        assert!(dbg.contains("***redacted***"));
    }
}
