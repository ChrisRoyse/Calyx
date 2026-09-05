//! Live subprocess plumbing — the shared spine for the console's read-only,
//! hash-verified surfaces.
//!
//! A live command shells out to the pinned `calyx` binary with an EXACT argv,
//! captures the real stdout JSON, and returns it verbatim. There is NO free-form
//! shell input: the argv is built from validated tokens only. On a non-zero exit
//! the subprocess's own structured stderr envelope is forwarded verbatim with HTTP
//! 502 — the console shows exactly what the accepted code path said, never a
//! faked-green result.
//!
//! The public spawn/interpret helpers here are also the backend for the research
//! -chat tool runner ([`crate::chat::tools`]); the only HTTP route this module now
//! serves directly is the bounded opinion-excerpt readback
//! (`GET /api/opinion/{cx}/excerpt`).

use axum::{
    Json,
    extract::{FromRequest, Path, Query, Request, State, rejection::JsonRejection},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::config::Config;
use crate::error::{ApiError, ErrorCode};

/// Max bytes of a subprocess's stderr we forward when it is NOT valid JSON.
const MAX_RAW_STDERR: usize = 16 * 1024;

/// Max characters of an offending ledger-tail line echoed in an error message.
const MAX_LEDGER_LINE_SNIPPET: usize = 256;

/// A `Json<T>` extractor whose deserialization rejection is mapped into the
/// closed `{code,message,remediation}` envelope as `CALYX_LAWDEMO_BAD_REQUEST`
/// (HTTP 400) instead of axum's default plain-text 422. Every non-success
/// response on this service is the closed envelope — including a body that is
/// missing a field, has the wrong type, or is not JSON at all.
#[derive(Debug)]
pub struct ValidatedJson<T>(pub T);

impl<S, T> FromRequest<S> for ValidatedJson<T>
where
    S: Send + Sync,
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(ValidatedJson(value)),
            Err(rejection) => Err(ApiError::new(
                ErrorCode::BadRequest,
                format!(
                    "request body could not be deserialized into the expected shape: {}",
                    rejection.body_text()
                ),
            )),
        }
    }
}

/// A syntactically-safe answer/opinion id: ULID/hex-ish, never empty, never flag-like.
/// This prevents a caller from smuggling a `--flag` or path into the argv even
/// though the array form already blocks shell metacharacters. Reused as the cx_id
/// contract ([`valid_cx_id`]) and by the research-chat tool runner.
pub fn valid_answer_id(id: &str) -> bool {
    let id = id.trim();
    (8..=128).contains(&id.len())
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The default excerpt window for `GET /api/opinion/{cx_id}/excerpt`. Bounded so a card
/// read-on-demand can never dump a whole opinion — just enough to prove text is real.
const EXCERPT_DEFAULT_BYTES: u32 = 2400;
/// Hard ceiling on a requested excerpt window (a client-supplied `?max_bytes`).
const EXCERPT_MAX_BYTES: u32 = 8192;

/// A syntactically-safe cx_id (same contract as the chat tool `case_read`): 8..=128 chars
/// of `[A-Za-z0-9_-]`, never flag-like. Blocks argv smuggling; frontier `frontier:<...>`
/// refs (which carry a colon) are rejected here too — they have no readable text.
pub fn valid_cx_id(cx: &str) -> bool {
    valid_answer_id(cx)
}

/// `calyx readback case-text --vault <vault> --cx-id <cx> --max-bytes <n>`.
///
/// The excerpt endpoint reuses the EXACT retained-input decode the `case-text` CLI verb
/// uses: it follows the constellation's retained-input reference and verifies the bytes
/// against the stored hash (fail-closed on redacted/missing/mismatch). Read-only, scoped
/// to the ONE configured production vault, cx_id validated with the same arg rules.
pub fn build_excerpt_argv(cfg: &Config, cx_id: &str, max_bytes: u32) -> Vec<String> {
    let n = max_bytes.clamp(1, EXCERPT_MAX_BYTES);
    vec![
        "readback".into(),
        "case-text".into(),
        "--vault".into(),
        cfg.vault.clone(),
        "--cx-id".into(),
        cx_id.trim().to_owned(),
        "--max-bytes".into(),
        n.to_string(),
    ]
}

/// A non-zero exit forwards the subprocess's structured stderr envelope verbatim
/// under `upstream` (HTTP 502). Non-JSON stderr is wrapped as `raw_stderr`.
fn forward_upstream_error(stderr: &[u8]) -> ApiError {
    let upstream = match serde_json::from_slice::<Value>(stderr) {
        Ok(envelope) => envelope,
        Err(_) => {
            let mut text = String::from_utf8_lossy(stderr).into_owned();
            text.truncate(MAX_RAW_STDERR);
            json!({ "raw_stderr": text })
        }
    };
    ApiError::of(ErrorCode::LiveCommandFailed).with_upstream(upstream)
}

/// Interpret a finished single-JSON-document subprocess (kernel-answer,
/// reproduce, verify-chain): exit-0 stdout JSON → 200; non-zero → forward the
/// stderr envelope verbatim with 502; exit-0 non-JSON stdout → structured error.
pub(crate) fn interpret(
    argv: &[String],
    status_ok: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<Value, ApiError> {
    if status_ok {
        return serde_json::from_slice::<Value>(stdout).map_err(|_| {
            ApiError::new(
                ErrorCode::Internal,
                format!("live `{}` exited 0 but stdout was not JSON", argv[0]),
            )
        });
    }
    Err(forward_upstream_error(stderr))
}

/// Interpret a finished `ledger-tail` subprocess (the backend for the research-chat
/// `ledger_tail` tool). Unlike the single-document live commands, `calyx ledger-tail`
/// emits **JSONL**: line 1 is the verify summary object
/// (`{verified_count, verify_chain}`) and each subsequent line is one ledger
/// entry object (`{actor, entry_hash, kind, payload, prev_hash, seq}`). Success
/// yields `{verify: <first-line object>, entries: [<rest>]}`. A non-zero exit
/// forwards the stderr envelope (502). ANY non-JSON line on exit-0 is a
/// structured [`ErrorCode::Internal`] naming the offending line — fail closed,
/// never a half-parsed result.
pub(crate) fn interpret_ledger_tail(
    status_ok: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<Value, ApiError> {
    if !status_ok {
        return Err(forward_upstream_error(stderr));
    }

    let text = std::str::from_utf8(stdout).map_err(|_| {
        ApiError::new(
            ErrorCode::Internal,
            "live `ledger-tail` exited 0 but stdout was not UTF-8".to_owned(),
        )
    })?;

    let mut verify: Option<Value> = None;
    let mut entries: Vec<Value> = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue; // tolerate a trailing newline / blank separators
        }
        let value = serde_json::from_str::<Value>(line).map_err(|_| {
            let mut snippet = line.to_owned();
            snippet.truncate(MAX_LEDGER_LINE_SNIPPET);
            ApiError::new(
                ErrorCode::Internal,
                format!("live `ledger-tail` emitted a non-JSON line: {snippet:?}"),
            )
        })?;
        if verify.is_none() {
            verify = Some(value);
        } else {
            entries.push(value);
        }
    }

    match verify {
        Some(verify) => Ok(json!({ "verify": verify, "entries": entries })),
        None => Err(ApiError::new(
            ErrorCode::Internal,
            "live `ledger-tail` exited 0 but produced no output".to_owned(),
        )),
    }
}

/// Spawn the pinned binary with `argv`, enforce the default live timeout, and return
/// the raw finished output. A missing binary is [`ErrorCode::LiveUnavailable`]; a
/// timeout is [`ErrorCode::LiveTimeout`]. `CALYX_HOME` (and `ORT_DYLIB_PATH` when
/// configured) are set explicitly on the subprocess env.
///
/// This is the entry point used by the `/api/live/*` routes. Chat tools use the
/// same configured budget through [`spawn_live_with_timeout`].
pub(crate) async fn spawn_live(
    cfg: &Config,
    argv: &[String],
) -> Result<std::process::Output, ApiError> {
    spawn_live_with_timeout(cfg, argv, cfg.live_timeout).await
}

/// Like [`spawn_live`] but with an explicit time budget supplied by the caller.
pub(crate) async fn spawn_live_with_timeout(
    cfg: &Config,
    argv: &[String],
    timeout: std::time::Duration,
) -> Result<std::process::Output, ApiError> {
    let mut cmd = tokio::process::Command::new(&cfg.calyx_binary);
    cmd.args(argv)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    cfg.apply_subprocess_env(&mut cmd);

    match tokio::time::timeout(timeout, cmd.output()).await {
        Err(_elapsed) => {
            tracing::warn!("live `{}` exceeded {:?}", argv[0], timeout);
            Err(ApiError::of(ErrorCode::LiveTimeout))
        }
        Ok(Err(io_err)) => {
            tracing::error!("live spawn of {:?} failed: {io_err}", cfg.calyx_binary);
            Err(ApiError::new(
                ErrorCode::LiveUnavailable,
                "the pinned calyx binary could not be launched".to_owned(),
            ))
        }
        Ok(Ok(output)) => Ok(output),
    }
}

/// Spawn and interpret a single-JSON-document live command.
async fn run_live(cfg: &Config, argv: Vec<String>) -> Response {
    match spawn_live(cfg, &argv).await {
        Err(err) => err.into_response(),
        Ok(output) => match interpret(
            &argv,
            output.status.success(),
            &output.stdout,
            &output.stderr,
        ) {
            Ok(value) => Json(value).into_response(),
            Err(err) => err.into_response(),
        },
    }
}

/// Query for `GET /api/opinion/{cx_id}/excerpt`.
#[derive(Debug, Deserialize)]
pub struct ExcerptQuery {
    /// Window size in bytes (clamped to 1..=8192; default 2400).
    pub max_bytes: Option<u32>,
}

/// `GET /api/opinion/{cx_id}/excerpt` — a bounded, hash-verified opinion-text excerpt for
/// ONE cx_id, so a source card that was verified by existence (corpus_search/cite_check)
/// but never read in-episode can still show REAL retained text on demand. Fail-closed:
/// an invalid cx_id is a `CALYX_LAWDEMO_BAD_REQUEST`; a redacted/missing/mismatched blob
/// is the subprocess's own forwarded envelope (502). Only the configured vault is read.
pub async fn opinion_excerpt_handler(
    State(state): State<AppState>,
    Path(cx_id): Path<String>,
    Query(q): Query<ExcerptQuery>,
) -> Response {
    if !valid_cx_id(&cx_id) {
        return ApiError::new(
            ErrorCode::BadRequest,
            "cx_id must be an 8..=128 char alphanumeric/_/- token (an in-corpus opinion id; \
             out-of-corpus/frontier refs have no readable text)"
                .to_owned(),
        )
        .into_response();
    }
    let argv = build_excerpt_argv(
        &state.config,
        &cx_id,
        q.max_bytes.unwrap_or(EXCERPT_DEFAULT_BYTES),
    );
    run_live(&state.config, argv).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg() -> Config {
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
        let map: HashMap<String, String> = pairs
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        Config::from_map(&map).unwrap()
    }

    #[test]
    fn answer_id_validation_rejects_injection_and_flags() {
        assert!(valid_answer_id("01KXJQ6Y8SSREK6D8NPKYMJB9G"));
        assert!(valid_answer_id("8f9b8ff1e2ed614e732c0e0f4e1ac62b"));
        assert!(!valid_answer_id("--record"));
        assert!(!valid_answer_id("../../etc/passwd"));
        assert!(!valid_answer_id("id with space"));
        assert!(!valid_answer_id("short"));
        assert!(!valid_answer_id(""));
    }

    #[test]
    fn excerpt_argv_reuses_case_text_decode_and_clamps_window() {
        let cx = "3a1c7de290bb4c1187aa0e2f9d5c6b71";
        let argv = build_excerpt_argv(&cfg(), cx, 999_999);
        assert_eq!(
            argv,
            vec![
                "readback",
                "case-text",
                "--vault",
                "/zfs/hot/calyx/vaults/PROD",
                "--cx-id",
                cx,
                "--max-bytes",
                "8192", // clamped to the ceiling
            ]
        );
        // The default window is bounded.
        let def = build_excerpt_argv(&cfg(), cx, EXCERPT_DEFAULT_BYTES);
        assert_eq!(def.last().unwrap(), "2400");
    }

    #[test]
    fn excerpt_cx_id_validation_rejects_frontier_and_injection() {
        assert!(valid_cx_id("3a1c7de290bb4c1187aa0e2f9d5c6b71"));
        // A frontier ref (colon) has no readable text — rejected before any spawn.
        assert!(!valid_cx_id("frontier:9f7b1c3ad2e04455aa8f0b6612cd7788"));
        assert!(!valid_cx_id("--record"));
        assert!(!valid_cx_id("../../etc/passwd"));
        assert!(!valid_cx_id("short"));
    }

    #[test]
    fn nonzero_exit_forwards_structured_stderr_verbatim() {
        let argv = vec!["kernel-answer".to_owned()];
        let stderr = br#"{"code":"CALYX_RESIDENT_UNREACHABLE","message":"down"}"#;
        let err = interpret(&argv, false, b"", stderr).unwrap_err();
        assert_eq!(err.code(), ErrorCode::LiveCommandFailed);
        let body = err.body();
        assert_eq!(body["upstream"]["code"], "CALYX_RESIDENT_UNREACHABLE");
    }

    #[test]
    fn nonzero_exit_with_plain_stderr_is_wrapped_as_raw() {
        let argv = vec!["verify-chain".to_owned()];
        let err = interpret(&argv, false, b"", b"boom not json").unwrap_err();
        assert_eq!(err.body()["upstream"]["raw_stderr"], "boom not json");
    }

    #[test]
    fn zero_exit_json_stdout_is_returned() {
        let argv = vec!["kernel-answer".to_owned()];
        let out = interpret(&argv, true, br#"{"status":"grounded"}"#, b"").unwrap();
        assert_eq!(out["status"], "grounded");
    }

    // --- Defect 3: ledger-tail is JSONL, not one JSON document. ----------------

    /// A real-shaped ledger-tail sample: line 1 is the verify summary, then one
    /// object per entry (fields per the pinned binary output in issue #1958).
    const LEDGER_TAIL_JSONL: &[u8] = br#"{"verified_count":77236,"verify_chain":"Intact"}
{"actor":"resident","entry_hash":"9f2c8d41ab","kind":"answer","payload":{"answer_id":"01KXJQ6Y8SSREK6D8NPKYMJB9G"},"prev_hash":"000000abcd","seq":77235}
{"actor":"resident","entry_hash":"a1b2c3d4e5","kind":"reproduce","payload":{"anchor_kind":"kernel_node"},"prev_hash":"9f2c8d41ab","seq":77236}
"#;

    #[test]
    fn ledger_tail_parses_jsonl_into_verify_and_entries() {
        let out = interpret_ledger_tail(true, LEDGER_TAIL_JSONL, b"").unwrap();
        assert_eq!(out["verify"]["verified_count"], 77236);
        assert_eq!(out["verify"]["verify_chain"], "Intact");
        let entries = out["entries"].as_array().expect("entries array");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["seq"], 77235);
        assert_eq!(entries[0]["kind"], "answer");
        assert_eq!(entries[0]["entry_hash"], "9f2c8d41ab");
        assert_eq!(entries[1]["kind"], "reproduce");
        assert_eq!(entries[1]["payload"]["anchor_kind"], "kernel_node");
    }

    #[test]
    fn ledger_tail_verify_only_yields_empty_entries() {
        let stdout = br#"{"verified_count":77236,"verify_chain":"Intact"}
"#;
        let out = interpret_ledger_tail(true, stdout, b"").unwrap();
        assert_eq!(out["verify"]["verified_count"], 77236);
        assert_eq!(out["entries"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn ledger_tail_non_json_line_fails_closed_naming_the_line() {
        let stdout = br#"{"verified_count":77236,"verify_chain":"Intact"}
this is not json
"#;
        let err = interpret_ledger_tail(true, stdout, b"").unwrap_err();
        assert_eq!(err.code(), ErrorCode::Internal);
        let msg = err.body()["message"].as_str().unwrap().to_owned();
        assert!(msg.contains("non-JSON line"), "got: {msg}");
        assert!(msg.contains("this is not json"), "got: {msg}");
    }

    #[test]
    fn ledger_tail_empty_stdout_is_internal_error() {
        let err = interpret_ledger_tail(true, b"", b"").unwrap_err();
        assert_eq!(err.code(), ErrorCode::Internal);
        assert!(
            err.body()["message"]
                .as_str()
                .unwrap()
                .contains("produced no output")
        );
    }

    #[test]
    fn ledger_tail_nonzero_exit_forwards_upstream_stderr() {
        let stderr = br#"{"code":"CALYX_CLI_USAGE_ERROR","message":"CALYX_HOME is required for vault commands"}"#;
        let err = interpret_ledger_tail(false, b"", stderr).unwrap_err();
        assert_eq!(err.code(), ErrorCode::LiveCommandFailed);
        assert_eq!(err.body()["upstream"]["code"], "CALYX_CLI_USAGE_ERROR");
    }

    // --- Defect 2: malformed JSON body maps to the closed envelope. ------------
    //
    // `ValidatedJson` is the shared JSON extractor used by the surviving `/api/chat`
    // routes; these exercise it directly against a minimal probe body so the closed
    // BAD_REQUEST envelope contract is proven independent of any one route's shape.

    #[derive(Debug, serde::Deserialize)]
    struct ProbeBody {
        field: String,
    }

    #[tokio::test]
    async fn malformed_body_maps_to_bad_request_envelope() {
        use axum::body::Body;
        // Missing the required `field` → axum would emit a plain 422; ValidatedJson
        // must convert it to the closed BAD_REQUEST envelope.
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/probe")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"other":"value"}"#))
            .unwrap();
        let err = ValidatedJson::<ProbeBody>::from_request(req, &())
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::BadRequest);
        let body = err.body();
        assert_eq!(body["code"], "CALYX_LAWDEMO_BAD_REQUEST");
        assert!(
            body["message"]
                .as_str()
                .unwrap()
                .contains("deserialized into the expected shape"),
            "got: {}",
            body["message"]
        );
        assert!(body["remediation"].is_string());
    }

    #[tokio::test]
    async fn non_json_body_maps_to_bad_request_envelope() {
        use axum::body::Body;
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/probe")
            .header("content-type", "application/json")
            .body(Body::from("not json at all"))
            .unwrap();
        let err = ValidatedJson::<ProbeBody>::from_request(req, &())
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::BadRequest);
    }

    #[tokio::test]
    async fn valid_body_is_accepted_by_validated_json() {
        use axum::body::Body;
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/probe")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"field":"ok"}"#))
            .unwrap();
        let ValidatedJson(body) = ValidatedJson::<ProbeBody>::from_request(req, &())
            .await
            .expect("valid body accepted");
        assert_eq!(body.field, "ok");
    }
}
