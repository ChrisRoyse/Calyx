use super::*;

/// Is this one of the GPU-backed (calyxd) routes that gets the tighter body cap
/// and rate-limit bucket?
fn is_gpu_route(path: &str) -> bool {
    matches!(path, "/v1/measure" | "/v1/search" | "/v1/guard")
}

/// A simple global token-bucket per route. "Global" (not per-IP) is the correct
/// key here: the service is loopback-only behind a reverse proxy, so every
/// request shares one peer — the bucket protects the single GPU from pile-up,
/// not individual clients (that is the proxy/WAF's job). Refill is wall-clock
/// based via a monotonic [`Instant`].
struct Bucket {
    tokens: f64,
    last: Instant,
}

/// Per-request resource limits: a route-keyed token-bucket rate limiter (GPU
/// routes get a tighter bucket) plus the request timeout. Carried as shared
/// state so tests can inject a tiny limit / short timeout deterministically.
pub struct Guardrails {
    capacity: f64,
    refill_per_sec: f64,
    gpu_capacity: f64,
    gpu_refill_per_sec: f64,
    timeout: Duration,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl Guardrails {
    /// Construct explicit guardrails (used by tests to force a tiny limit /
    /// short timeout).
    pub fn new(
        capacity: f64,
        refill_per_sec: f64,
        gpu_capacity: f64,
        gpu_refill_per_sec: f64,
        timeout: Duration,
    ) -> Self {
        Self {
            capacity,
            refill_per_sec,
            gpu_capacity,
            gpu_refill_per_sec,
            timeout,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Production limits: generous on light read routes, tight on GPU routes,
    /// with the standard [`REQUEST_TIMEOUT`].
    pub fn production() -> Self {
        Self::new(60.0, 30.0, 8.0, 2.0, REQUEST_TIMEOUT)
    }

    /// Take one token for `path`. Returns `true` if allowed, `false` if the
    /// bucket is empty (rate-limited).
    fn allow(&self, path: &str) -> bool {
        let (cap, refill) = if is_gpu_route(path) {
            (self.gpu_capacity, self.gpu_refill_per_sec)
        } else {
            (self.capacity, self.refill_per_sec)
        };
        let now = Instant::now();
        let mut buckets = self.buckets.lock().expect("rate-limiter mutex poisoned");
        let bucket = buckets.entry(path.to_owned()).or_insert(Bucket {
            tokens: cap,
            last: now,
        });
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * refill).min(cap);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Build the application with the production guardrails.
pub fn app() -> Router {
    build_app(Arc::new(Guardrails::production()))
}

/// Build the application with explicit guardrails (testable injection). Wires
/// the route surface, the enveloped 404 + 405 fallbacks, the resource
/// [`guardrails`] (body cap + rate limit + timeout), and the panic-catch layer.
pub fn build_app(limiter: Arc<Guardrails>) -> Router {
    routes()
        .fallback(fallback_404)
        .method_not_allowed_fallback(fallback_405)
        .layer(middleware::from_fn_with_state(limiter, guardrails))
        .layer(panic_catch_layer())
}

/// The read-only route surface fully scaffolded (`measure` + `provenance` to
/// 501). The wired variants are [`build_app_with_provenance`] and
/// [`build_app_with_measure_and_provenance`].
fn routes() -> Router {
    routes_base()
        .route("/v1/health", get(health))
        .route("/v1/measure", post(not_implemented))
        .route("/v1/search", post(not_implemented))
        .route("/v1/guard", post(not_implemented))
        .route("/v1/kernel", get(not_implemented))
        .route("/v1/provenance/{id}", get(provenance_stub))
}

/// The shared base with NO route attached — every endpoint (including the now
/// stateful `/v1/health`) is added by the scaffolded or wired builder, so each
/// can choose its handler without a route-overlap panic. The deployed origin
/// uses the stateful [`health_full`] (real gpu/vault/panelVersion for the edge
/// circuit breaker); the scaffold uses the stateless [`health`].
pub(super) fn routes_base() -> Router {
    Router::new()
}

/// Per-request resource guardrails. Order: rate limit (cheapest reject) → body
/// cap (route-aware) → timeout around the handler.
pub async fn guardrails(
    State(limiter): State<Arc<Guardrails>>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_owned();

    if !limiter.allow(&path) {
        let mut resp = ApiError::new(
            ErrorCode::RateLimited,
            format!("rate limit exceeded for {path}"),
        )
        .into_response();
        resp.headers_mut()
            .insert(header::RETRY_AFTER, header::HeaderValue::from_static("1"));
        return resp;
    }

    let cap = if is_gpu_route(&path) {
        MAX_GPU_BODY_BYTES
    } else {
        MAX_BODY_BYTES
    };
    let (parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, cap).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return ApiError::new(
                ErrorCode::PayloadTooLarge,
                format!("request body exceeds the {cap}-byte limit for {path}"),
            )
            .into_response();
        }
    };
    let req = Request::from_parts(parts, Body::from(bytes));

    match tokio::time::timeout(limiter.timeout, next.run(req)).await {
        Ok(resp) => resp,
        Err(_elapsed) => {
            tracing::warn!(
                "CALYX_WEB_API_TIMEOUT: request to {path} exceeded {:?}",
                limiter.timeout
            );
            ApiError::of(ErrorCode::Timeout).into_response()
        }
    }
}
