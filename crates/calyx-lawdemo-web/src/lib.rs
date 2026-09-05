//! calyx-lawdemo-web — the Research Console frontend for the canonical Ohio law
//! corpus (Cuyahoga Eighth District plus Ohio Supreme Court opinions).
//!
//! A small axum service, loopback ONLY (`127.0.0.1:8600` by default; a
//! non-loopback bind is refused at startup), serving:
//!   * `/console` (and `/`, which serves the same page) — the single-file
//!     Research Console (inline CSS/JS, embedded via `include_str!`; no npm, no
//!     CDN, no external fonts — it renders perfectly over an SSH tunnel with no
//!     internet).
//!   * a thin JSON API: the DeepSeek research-chat SSE stream (`/api/chat`) and
//!     its cancel control, aggregate readiness (`/api/health`), the SHA-256
//!     -verified corpus-census artifact (`/api/demo/4`), and the bounded, hash
//!     -verified opinion-excerpt readback (`/api/opinion/{cx}/excerpt`).
//!
//! Repo doctrine holds: no mock data on the production path (artifact bytes are
//! SHA-verified real artifacts; live output is the real stdout of the accepted
//! code path), and every failure is the closed `{code,message,remediation}`
//! envelope — never a faked-green result.

use std::any::Any;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::{
    Router,
    http::{HeaderName, HeaderValue, Method, Uri, header::CACHE_CONTROL},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::set_header::SetResponseHeaderLayer;

pub mod artifact;
pub mod chat;
pub mod config;
pub mod error;
pub mod health;
pub mod live;
pub mod production;

/// Reality contract for the Research Console Inspector, driven by the REAL live-SSE
/// fixtures under `tests/fixtures/console/`. Test-only (autotests are disabled, so this
/// runs as part of the lib unit-test binary).
#[cfg(test)]
mod console_live_sse_tests;

pub use config::Config;
use error::{ApiError, ErrorCode};

/// The full-page Research Console — the single user-facing surface, served at both
/// `GET /console` and `GET /` (same body). A single-file page modelled on the
/// ChatGPT / Claude.ai anatomy (left rail · center stream · right Inspector) that
/// drives the grounded chat backend as a full Calyx capability showcase. Embedded at
/// compile time via `include_str!` so the running binary is fully self-contained:
/// inline CSS/JS, no external asset, nothing fetched over the tunnel.
const CONSOLE_HTML: &str = include_str!("web/console.html");

/// Shared application state: the validated configuration behind an `Arc`.
#[derive(Clone)]
pub struct AppState {
    /// The immutable runtime configuration.
    pub config: Arc<Config>,
}

impl AppState {
    /// Wrap a validated [`Config`] as shared state.
    pub fn new(config: Config) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

/// Build the full application router over the given state.
pub fn app(state: AppState) -> Router {
    Router::new()
        // `/` and `/console` serve the SAME Research Console body, so the old demo
        // URL can never render a stale page — it just renders the console.
        .route("/", get(serve_console))
        .route("/console", get(serve_console))
        .route("/api/health", get(health::health_handler))
        .route("/api/demo/{demo}", get(artifact::demo_handler))
        .route(
            "/api/opinion/{cx_id}/excerpt",
            get(live::opinion_excerpt_handler),
        )
        .route("/api/chat", post(chat::chat_handler))
        // The REATTACH route (#2061): rejoin a running or recently finished episode at
        // any `from_seq`. This is what makes a frozen/discarded background tab, a
        // dropped tunnel, or a slept laptop recoverable instead of terminal.
        .route("/api/chat/stream", get(chat::chat_stream_handler))
        .route("/api/chat/cancel", post(chat::chat_cancel_handler))
        .fallback(fallback_404)
        .method_not_allowed_fallback(fallback_405)
        .layer(panic_catch_layer())
        // Applied OUTERMOST so it stamps every response the service can emit — the
        // embedded console HTML, every JSON API body, the chat SSE stream, the
        // closed error envelope, and even a caught-panic 500 — and, being on the
        // router itself, any route added later inherits it by construction. This is
        // the "never again" property for #1959: no handler can forget the header.
        .layer(static_header_layer(
            "strict-transport-security",
            "max-age=31536000",
        ))
        .layer(static_header_layer(
            "permissions-policy",
            "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
        ))
        .layer(static_header_layer(
            "referrer-policy",
            "strict-origin-when-cross-origin",
        ))
        .layer(static_header_layer("x-content-type-options", "nosniff"))
        .layer(static_header_layer("x-frame-options", "DENY"))
        .layer(static_header_layer("x-xss-protection", "0"))
        .layer(content_security_policy_layer())
        .layer(no_store_cache_layer())
        .with_state(state)
}

/// The single structural source of the `Cache-Control` header for the whole
/// service (#1959).
///
/// The demo surface has no content-hashed asset pipeline: the Research Console is
/// embedded via `include_str!` and served from memory, and every other response is
/// a freshly computed JSON/SSE body. Nothing is served from disk with an
/// `ETag`/`Last-Modified` validator, so `no-cache` (revalidate) would have nothing
/// to revalidate against — without a validator the browser must refetch the full
/// body regardless. `no-store` is therefore the honest, correct directive: it
/// forbids the browser and every intermediary from retaining ANY copy, so a
/// redeploy can never leave a stale console (or a stale API shape) in front of an
/// investor mid-pitch.
///
/// `overriding` (not `if_not_present`) guarantees the directive wins even if a
/// future handler sets its own `Cache-Control` — the property holds by
/// construction, not by convention.
pub fn no_store_cache_layer() -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::overriding(CACHE_CONTROL, HeaderValue::from_static("no-store"))
}

/// Construct an overriding policy header layer from compile-time literals.
///
/// These layers sit outside every route and the panic catcher, so security
/// policy cannot silently disappear from a newly added endpoint or error path.
fn static_header_layer(
    name: &'static str,
    value: &'static str,
) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::overriding(
        HeaderName::from_static(name),
        HeaderValue::from_static(value),
    )
}

/// The console's CSP is derived from the exact embedded `<style>` and `<script>`
/// bytes. This keeps the executable policy strict without a hand-maintained hash
/// that could drift and blank the UI after a legitimate source edit.
fn content_security_policy_layer() -> SetResponseHeaderLayer<HeaderValue> {
    static POLICY: OnceLock<HeaderValue> = OnceLock::new();
    let value = POLICY.get_or_init(|| {
        let style = embedded_tag_body(CONSOLE_HTML, "style");
        let script = embedded_tag_body(CONSOLE_HTML, "script");
        let style_hash = STANDARD.encode(Sha256::digest(style.as_bytes()));
        let script_hash = STANDARD.encode(Sha256::digest(script.as_bytes()));
        let policy = format!(
            "default-src 'none'; base-uri 'none'; object-src 'none'; \
             frame-ancestors 'none'; form-action 'self'; connect-src 'self'; \
             img-src 'self' data:; style-src 'unsafe-inline'; \
             style-src-elem 'sha256-{style_hash}'; style-src-attr 'unsafe-inline'; \
             script-src 'sha256-{script_hash}'"
        );
        HeaderValue::from_str(&policy)
            .expect("embedded console CSP must be a valid HTTP header value")
    });
    SetResponseHeaderLayer::overriding(
        HeaderName::from_static("content-security-policy"),
        value.clone(),
    )
}

/// Return the one inline tag body used by the single-file console.
///
/// Missing, duplicate, or malformed tags are build/runtime invariants and fail
/// loudly instead of weakening CSP with a permissive fallback.
fn embedded_tag_body(document: &'static str, tag: &str) -> &'static str {
    let opening = format!("<{tag}>");
    let closing = format!("</{tag}>");
    let (_, after_opening) = document
        .split_once(&opening)
        .unwrap_or_else(|| panic!("embedded console is missing {opening}"));
    let (body, after_closing) = after_opening
        .split_once(&closing)
        .unwrap_or_else(|| panic!("embedded console is missing {closing}"));
    assert!(
        !after_closing.contains(&opening),
        "embedded console contains more than one {opening}"
    );
    body
}

/// Spawn the episode reaper (#2061) — the bound on orphaned DeepSeek spend and
/// orphaned GPU search permits.
///
/// A browser cannot be relied on to say goodbye: a closed tab, a discarded page, a
/// killed mobile app, and a dropped tunnel all vanish without a `pagehide`. So episode
/// liveness is decided by ATTACHMENT, not by any farewell message. Every second this
/// cancels episodes that have had no attached client for the configured grace period,
/// evicts finished episodes past their retention window, and loudly evicts any
/// registration that outlived every budget it has.
///
/// Returns `None` when chat is disabled — there are no episodes to reap.
pub fn spawn_episode_reaper(config: &Config) -> Option<tokio::task::JoinHandle<()>> {
    let chat = config.chat.as_ref()?;
    let grace = chat.client_grace;
    let retention = chat.episode_retention;
    // A registration still running well past the whole wall budget is a leak, not a
    // slow episode. The margin is generous so a legitimately slow terminal event is
    // never mistaken for one.
    let leak_after = chat.wall_timeout + Duration::from_secs(120);
    tracing::info!(
        grace_secs = grace.as_secs(),
        retention_secs = retention.as_secs(),
        leak_after_secs = leak_after.as_secs(),
        "episode reaper started"
    );
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(chat::REAP_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let (cancelled, evicted) = chat::reap(grace, retention, leak_after);
            if cancelled > 0 || evicted > 0 {
                tracing::info!(cancelled, evicted, "episode reaper pass");
            }
        }
    }))
}

/// Serve the embedded Research Console page (the body for both `/` and `/console`).
async fn serve_console() -> Html<&'static str> {
    Html(CONSOLE_HTML)
}

/// 404 — no route matched. Echoes method + PATH only (never the query string).
async fn fallback_404(method: Method, uri: Uri) -> ApiError {
    ApiError::new(
        ErrorCode::NotFound,
        format!("no route for {method} {}", uri.path()),
    )
}

/// 405 — the path exists but not for this method.
async fn fallback_405(method: Method, uri: Uri) -> ApiError {
    ApiError::new(
        ErrorCode::MethodNotAllowed,
        format!("{method} is not supported for {}", uri.path()),
    )
}

/// The panic-catching layer: any handler panic becomes a generic
/// `CALYX_LAWDEMO_INTERNAL` 500. The panic detail is logged, never surfaced.
pub fn panic_catch_layer() -> CatchPanicLayer<fn(Box<dyn Any + Send + 'static>) -> Response> {
    CatchPanicLayer::custom(on_panic as fn(Box<dyn Any + Send + 'static>) -> Response)
}

fn on_panic(payload: Box<dyn Any + Send + 'static>) -> Response {
    let detail = if let Some(s) = payload.downcast_ref::<&str>() {
        *s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.as_str()
    } else {
        "non-string panic payload"
    };
    tracing::error!("CALYX_LAWDEMO_INTERNAL: a request handler panicked: {detail}");
    ApiError::of(ErrorCode::Internal).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Load-bearing attribute patterns that would fetch from an external host. The
    /// Research Console must render fully offline over a tunnel.
    const EXTERNAL_RESOURCE_NEEDLES: [&str; 11] = [
        "src=\"http",
        "src='http",
        "href=\"http",
        "href='http",
        "@import",
        "//cdn",
        "googleapis",
        "unpkg",
        "jsdelivr",
        "<script src",
        "<link rel=\"stylesheet\"",
    ];

    #[test]
    fn embedded_console_is_present_and_titled() {
        assert!(CONSOLE_HTML.contains("<title>Leapable Law"));
        // It drives the same reused chat backend + health endpoint.
        assert!(CONSOLE_HTML.contains("/api/chat"));
        assert!(CONSOLE_HTML.contains("/api/health"));
    }

    #[test]
    fn embedded_console_loads_no_external_resource() {
        for needle in EXTERNAL_RESOURCE_NEEDLES {
            assert!(
                !CONSOLE_HTML.contains(needle),
                "Console must not reference an external resource ({needle:?})"
            );
        }
    }

    #[test]
    fn console_lens_tile_reads_live_resident_warmed_count_not_a_hardcoded_number() {
        // Defect 2: the status-strip "learned lenses warmed" tile must bind to the live
        // resident readback (/api/health resident.warmed_lens_count == 9 when slot 12 is
        // parked), never the stale hardcoded "10" that mismatched the resident.
        assert!(
            CONSOLE_HTML.contains("resident.warmed_lens_count")
                || CONSOLE_HTML.contains("warmed_lens_count"),
            "the lens tile must read resident.warmed_lens_count from health"
        );
        // The stale hardcoded fallback and the per-thread search override that drove the
        // wrong "10 warmed" number must both be gone from the status tile.
        assert!(
            !CONSOLE_HTML.contains("lensProvenance.lens_count : null"),
            "the status tile must not derive the warmed count from a per-query search override"
        );
        assert!(
            !CONSOLE_HTML.contains(": \"10\") + \"</b> learned lenses"),
            "the lens tile must not fall back to a hardcoded '10' warmed count"
        );
        // The tile still labels the count as the resident's WARMED lenses.
        assert!(CONSOLE_HTML.contains("learned lenses "));
    }

    #[test]
    fn console_stream_has_a_scroll_container_contract() {
        // #1978: the chat stream must scroll independently. The root-cause fix pins the
        // grid row (so the row cannot grow past the viewport and clip via overflow:hidden)
        // and gives .main min-height:0 so .stream-wrap owns the overflow. The scroll
        // container is keyboard-focusable and testid-tagged for browser-level FSV.
        assert!(
            CONSOLE_HTML.contains("grid-template-rows: minmax(0, 1fr)"),
            "the app grid row must be pinned so the stream scrolls instead of clipping"
        );
        assert!(
            CONSOLE_HTML.contains(
                ".main { display: flex; flex-direction: column; min-width: 0; min-height: 0;"
            ),
            ".main must set min-height:0 so the flex column can shrink and the stream scrolls"
        );
        // The scroll container: testid + focusable (keyboard PgUp/PgDn/Home/End) + overflow.
        assert!(
            CONSOLE_HTML.contains("data-testid=\"stream-scroll\""),
            "the scroll container must be tagged for browser-level scroll FSV"
        );
        assert!(
            CONSOLE_HTML
                .contains("id=\"stream-wrap\" data-testid=\"stream-scroll\" tabindex=\"0\""),
            "the scroll container must be keyboard-focusable (tabindex) for PgUp/PgDn/Home/End"
        );
        assert!(
            CONSOLE_HTML.contains(".stream-wrap { flex: 1; overflow-y: auto; min-height: 0;"),
            "the stream container must own the vertical overflow"
        );
        // The jump-to-latest affordance (auto-follow release) is present and testid-tagged.
        assert!(
            CONSOLE_HTML.contains("data-testid=\"jump-to-latest\""),
            "the jump-to-latest affordance must be tagged for FSV"
        );
        // The Inspector body scrolls independently of the stream.
        assert!(
            CONSOLE_HTML.contains(".insp-body { flex: 1; overflow-y: auto;"),
            "the Inspector body must scroll independently"
        );
    }

    #[test]
    fn console_step_pill_binds_the_real_failure_code() {
        // #1977: a failed tool step must surface the real CALYX_* code + short reason
        // bound from the tool_result payload — never a bare "unavailable, adapted".
        // The tool_result handler lifts the failure via failureReason(...) and buildStep
        // renders the code (data-code) + reason (.serr) with a step-pill testid.
        assert!(
            CONSOLE_HTML.contains("const failure = d.ok ? null : failureReason(d.payload"),
            "the tool_result handler must bind the failure reason for every failed tool"
        );
        assert!(
            CONSOLE_HTML.contains("step.setAttribute(\"data-code\", s.failure.code)"),
            "a failed step pill must expose the real CALYX_* code as data-code"
        );
        assert!(
            CONSOLE_HTML.contains("class: \"serr\"")
                && CONSOLE_HTML.contains("class: \"ecode mono\""),
            "a failed step pill must render the code + short reason detail line"
        );
        assert!(
            CONSOLE_HTML.contains("\"data-testid\": \"step-pill\""),
            "step pills must be testid-tagged for browser-level FSV"
        );
        // Failed-pill rendering is covered by real live-SSE fixtures. Production HTML
        // must not carry a recorded episode that can simulate health when chat is down.
        assert!(
            !CONSOLE_HTML.contains("replayDemo") && !CONSOLE_HTML.contains("ep-demo"),
            "the console must not embed or expose a recorded substitute episode"
        );
        assert!(
            CONSOLE_HTML.contains("Set LAWDEMO_WEB_CHAT_ENABLED=1 and restart the web service."),
            "disabled chat must expose a direct operational remediation"
        );
    }

    #[test]
    fn console_in_text_citation_chip_links_to_its_source_card() {
        // #1977 defect 3: a clicked in-text `[cx:…]` chip must LAND on its Inspector
        // source card — open Sources, scroll to + highlight the card whose data-cx is the
        // FULL cx_id — with a Provenance chain-anchor fallback when the cx isn't a Source.
        assert!(
            CONSOLE_HTML.contains("chip.addEventListener(\"click\", () => focusCitationCard(cx))"),
            "a cx chip click must invoke the chip→card focus path (not a bare tab open)"
        );
        assert!(
            CONSOLE_HTML.contains("function focusCitationCard(cx)"),
            "the chip→card focus function must exist"
        );
        // It keys on the FULL cx via data-cx on source cards, and highlights the landing.
        assert!(
            CONSOLE_HTML.contains(".src-card[data-cx=\"' + cx + '\"]"),
            "the focus path must locate the source card by full cx_id"
        );
        assert!(
            CONSOLE_HTML.contains("\"data-testid\": \"source-card\"")
                && CONSOLE_HTML.contains("\"data-cx\": c.cx_id"),
            "source cards must be testid-tagged and carry their full cx_id"
        );
        assert!(
            CONSOLE_HTML.contains("classList.add(\"src-focus\")"),
            "the landing card must be visibly highlighted"
        );
        // Fallback to the Provenance chain anchor (also data-cx keyed) when not a Source.
        assert!(
            CONSOLE_HTML.contains(".prov-anchor[data-cx=\"' + cx + '\"]")
                && CONSOLE_HTML.contains("\"data-testid\": \"prov-anchor\""),
            "a chip whose cx isn't in Sources must fall back to its Provenance anchor"
        );
    }

    #[test]
    fn console_source_card_is_never_contentless() {
        // #1977 defect 5: a source verified by existence (corpus_search/cite_check) but
        // never read in-episode must NOT render blank — it shows what the verification
        // proved, an honest explainer, and a read-on-demand excerpt affordance that hits
        // the bounded, hash-verified excerpt endpoint.
        assert!(
            CONSOLE_HTML.contains("function sourceEvidenceBlock(c)")
                && CONSOLE_HTML.contains("Existence proven by "),
            "a contentless verified card must state what the verification proved"
        );
        assert!(
            CONSOLE_HTML.contains("Full opinion text was not read during this episode."),
            "the honest explainer line must be present"
        );
        assert!(
            CONSOLE_HTML.contains("function attachExcerptLoader(card, cx, question)")
                && CONSOLE_HTML.contains("\"data-testid\": \"load-excerpt\""),
            "a read-on-demand excerpt loader affordance must be present + testid-tagged"
        );
        assert!(
            CONSOLE_HTML.contains("\"/api/opinion/\" + encodeURIComponent(cx) + \"/excerpt\""),
            "the loader must call the bounded hash-verified excerpt endpoint"
        );
        // The chip→card path auto-triggers the loader so a click always lands on content.
        assert!(
            CONSOLE_HTML
                .contains("if (loader && !card.querySelector(\".excerpt\")) loader.click()"),
            "landing on a search-verified card must auto-load its excerpt"
        );
    }

    /// #2061: the console must survive losing its stream. These are the load-bearing
    /// bindings of that contract — if any of them is edited away, a backgrounded tab
    /// silently goes back to losing the episode, which is exactly the defect that was
    /// invisible before.
    #[test]
    fn console_stream_is_resumable_stoppable_and_stops_on_new_chat() {
        // (1) It tracks the per-event `seq` from the SSE `id:` field...
        assert!(
            CONSOLE_HTML.contains("else if (line.startsWith(\"id:\")) id = line.slice(3).trim()"),
            "the frame parser must read the SSE id: field that carries the resume point"
        );
        // ...and rejoins the episode at that point when the stream breaks.
        assert!(
            CONSOLE_HTML.contains("RESUME.route + \"?episode_id=\"")
                && CONSOLE_HTML.contains("\"&from_seq=\" + (t.lastSeq || 0)"),
            "a broken stream must be rejoined at the last seq the client holds"
        );
        assert!(
            CONSOLE_HTML.contains("async function driveEpisode(t, response)")
                && CONSOLE_HTML.contains("if (t.terminal) break;"),
            "the driver must keep reattaching until a TERMINAL event, not until the socket ends"
        );
        // (2) Coming back from hidden/frozen must trigger the reattach immediately.
        assert!(
            CONSOLE_HTML.contains("[\"visibilitychange\", \"resume\"]")
                && CONSOLE_HTML.contains("wakeReattach"),
            "returning from hidden/frozen must wake the reattach instead of waiting out the backoff"
        );
        // (3) An episode's lifetime is ONE page context. A reload deliberately ends
        // the research (pagehide stops the agent), so nothing may persist the episode
        // handle across page loads and try to resurrect it.
        assert!(
            !CONSOLE_HTML.contains("sessionStorage") && !CONSOLE_HTML.contains("localStorage"),
            "the episode handle must not outlive its page context"
        );
        // (4) Stop is CONFIRMED and does not tear the stream down, so the server's own
        // terminal `cancelled` event is the proof the agent stopped.
        assert!(
            CONSOLE_HTML.contains("CALYX_LAWDEMO_CHAT_CANCEL_ACK"),
            "the cancel acknowledgement must be read, not fired and forgotten"
        );
        assert!(
            CONSOLE_HTML.contains("t.stopped = false;")
                && CONSOLE_HTML.contains("Stop could not be delivered to the server"),
            "a cancel that could not be delivered must be surfaced in the UI, not only the console"
        );
        // (5) New chat stops the running agent first.
        assert!(
            CONSOLE_HTML.contains(
                "$(\"#new-chat\").addEventListener(\"click\", () => { startNewChat(); });"
            ) && CONSOLE_HTML.contains("await interruptEpisode(\"new-chat\", t);"),
            "New chat must stop the running episode before clearing the view"
        );
        // (6) A true tab termination cancels immediately; a bfcache/freeze must NOT.
        assert!(
            CONSOLE_HTML.contains("if (event.persisted) return;")
                && CONSOLE_HTML.contains("navigator.sendBeacon(\"/api/chat/cancel\""),
            "pagehide must cancel on termination only, never on a freeze the page returns from"
        );
    }

    /// The console is the ONLY user-facing page and is reachable at BOTH `/console`
    /// and `/` (the old demo URL now serves the same console body, so it can never
    /// render a stale demo page). Both must return the HTML console with 200.
    #[tokio::test]
    async fn console_is_served_at_console_and_root() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        for path in ["/console", "/"] {
            let state = AppState::new(crate::config::Config::from_map(&test_env()).unwrap());
            let response = app(state)
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path} must serve 200");
            let ct = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default();
            assert!(ct.contains("text/html"), "{path} content-type was {ct:?}");
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let text = std::str::from_utf8(&body).unwrap();
            assert!(
                text.contains("<title>Leapable Law") && text.contains("/api/chat"),
                "{path} must serve the Research Console body"
            );
        }
    }

    /// #1959: EVERY response the router can emit must carry `Cache-Control: no-store`,
    /// so a redeploy can never leave a stale console/API shape cached mid-demo. Proven
    /// across an HTML shell, a JSON API body, and the 404 fallback — the layer is on the
    /// router, so any future route inherits the header by construction.
    #[tokio::test]
    async fn every_response_is_no_store() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        // (path, method): the console at `/`, the console at `/console`, a JSON API
        // body, and the catch-all 404.
        let cases: [(&str, &str); 4] = [
            ("/", "GET"),
            ("/console", "GET"),
            ("/api/health", "GET"),
            ("/no/such/route", "GET"),
        ];
        for (path, method) in cases {
            let state = AppState::new(crate::config::Config::from_map(&test_env()).unwrap());
            let response = app(state)
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let cc = response
                .headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok());
            assert_eq!(
                cc,
                Some("no-store"),
                "{method} {path} must carry Cache-Control: no-store, got {cc:?}"
            );
        }
    }

    /// Minimal valid env for constructing a [`crate::config::Config`] in a route test.
    fn test_env() -> std::collections::HashMap<String, String> {
        let pairs = [
            ("LAWDEMO_WEB_CALYX_BINARY", "/nonexistent/calyx"),
            ("LAWDEMO_WEB_CALYX_HOME", "/var/lib/calyx"),
            ("LAWDEMO_WEB_RESIDENT_ADDR", "127.0.0.1:18401"),
            ("LAWDEMO_WEB_VAULT", "/zfs/PROD"),
            ("LAWDEMO_WEB_D1_VAULT", "/zfs/D1"),
            ("LAWDEMO_WEB_D1_KERNEL", "b8f96b5deea45b743229242283234f9e"),
            ("LAWDEMO_WEB_D2_VAULT", "/zfs/D2"),
            ("LAWDEMO_WEB_D2_KERNEL", "b38822aad594ab8c521b64683c118f36"),
            (
                "LAWDEMO_WEB_EVIDENCE_DIR",
                "crates/calyx-lawdemo-web/examples/evidence",
            ),
            ("LAWDEMO_WEB_DEMO4_SHA", &"d".repeat(64)),
        ];
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }
}
