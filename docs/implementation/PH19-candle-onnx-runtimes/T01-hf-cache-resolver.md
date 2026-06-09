# PH19 · T01 — HF cache resolver + weight path builder

| Field | Value |
|---|---|
| **Phase** | PH19 — candle-local + onnx runtimes |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/hf_cache.rs` (≤500) |
| **Depends on** | PH18 T01 (LensSpec exists) |
| **Axioms** | A4 |
| **PRD** | `dbprdplans/05 §2`, `13_STAGE3_REGISTRY.md §PH19` |

## Goal

Implement a canonical HF cache resolver that maps `(model_id, filename)` to
an absolute path under `$CALYX_HOME/.hf-cache/<model_id>/<filename>`. Both
`CandleLocalLens` and `OnnxLens` use this to find weight files on disk without
hard-coding paths. Token is read from `CALYX_HF_TOKEN` env var.

## Build (checklist of concrete, code-level steps)

- [x] `pub struct HfCacheConfig { pub root: PathBuf }` — default from
  `env::var("CALYX_HOME").map(|p| PathBuf::from(p).join(".hf-cache"))`.
- [x] `pub fn resolve(config: &HfCacheConfig, model_id: &str, filename: &str) -> Result<PathBuf>`:
  - sanitize `model_id` (replace `/` with `--` following HF hub convention).
  - build path `root/<sanitized_model_id>/<filename>`.
  - if file does not exist → `Err(CalyxError::runtime_unavailable(format!("weight
    file not found: {}; run HF Hub download or set CALYX_HOME correctly", path.display())))`.
  - return the `PathBuf`.
- [x] `pub fn hf_token() -> Option<String>`: reads `CALYX_HF_TOKEN` from env;
  returns `None` if absent (public models work without a token).
- [x] `HfCacheConfig::from_env() -> Result<Self>`: reads `CALYX_HOME`;
  returns `CALYX_REGISTRY_RUNTIME_UNAVAILABLE` if `CALYX_HOME` not set.
- [x] All paths constructed with `PathBuf::from` / `.join`; no string
  concatenation with `/`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `resolve` with a tmpdir as root + a pre-created dummy file →
  returns the expected `PathBuf`.
- [x] unit: `resolve` when file does not exist → `Err` with
  `"CALYX_REGISTRY_RUNTIME_UNAVAILABLE"` and path in message.
- [x] unit: model id `"BAAI/bge-m3"` sanitizes to `"BAAI--bge-m3"`.
- [x] edge (≥3): (1) `CALYX_HOME` not set → `from_env` fails with clear error;
  (2) `filename` with path separators sanitized or rejected; (3) model id
  with multiple slashes → correct double-dash encoding.
- [x] fail-closed: missing weight file → exact `"CALYX_REGISTRY_RUNTIME_UNAVAILABLE"`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `$CALYX_HOME/.hf-cache` directory on aiwonder filesystem
- **Readback:** `cargo test -p calyx-registry hf_cache -- --nocapture 2>&1`
  and `ls $CALYX_HOME/.hf-cache/` printed in a test
- **Prove:** resolve returns a path that points to an existing file in the
  `.hf-cache` directory; directory listing shows at least one model directory;
  attached to PH19 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH19 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
