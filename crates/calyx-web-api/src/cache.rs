use super::*;

// ---------------------------------------------------------------------------
// Bounded TTL response cache for the idempotent read endpoints (#1898)
// ---------------------------------------------------------------------------

/// One cached response: the EXACT serialized body bytes (so a hit replays
/// byte-for-byte) plus the monotonic instant it was stored (for TTL + `Age`).
pub(super) struct CacheEntry {
    body: Arc<[u8]>,
    stored: Instant,
}

/// A bounded, TTL-expiring in-memory response cache keyed by a request-derived
/// string.
///
/// `/v1/search` (by `(query,k,guard,fusion)`) and `/v1/provenance/{id}` (by id)
/// are PURE for a given vault/ledger state — provenance in particular does a
/// full `scan()` + `verify_chain()` on every call (#1898). A short TTL bounds
/// staleness against an out-of-band vault rebuild (which also restarts this
/// process and so clears the cache) while cutting that per-request work.
///
/// Bounded two ways so memory can never run away: an expired entry is dropped
/// the moment it is read, and an insertion beyond `capacity` evicts expired
/// entries first and then the oldest-stored key. A **zero TTL disables caching
/// entirely** (every request recomputes), so the layer can be turned off via
/// env without a code change. Never caches a non-200 / error response.
pub struct ResponseCache {
    ttl: Duration,
    capacity: usize,
    pub(super) entries: Mutex<HashMap<String, CacheEntry>>,
}

impl ResponseCache {
    /// Explicit construction (tests inject a tiny TTL/capacity deterministically).
    /// `capacity` is floored at 1 so the cache always holds at least one entry.
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            ttl,
            capacity: capacity.max(1),
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Build from the optional `CALYX_WEB_API_CACHE_TTL_SECS` (default 30, `0`
    /// disables) and `CALYX_WEB_API_CACHE_CAPACITY` (default 256) env vars. A
    /// present-but-unparseable value is a HARD error (fail loud — never a silent
    /// fallback to the default).
    pub fn from_env() -> Result<Self, String> {
        let ttl_secs = parse_env_u64("CALYX_WEB_API_CACHE_TTL_SECS", 30)?;
        let capacity = parse_env_u64("CALYX_WEB_API_CACHE_CAPACITY", 256)? as usize;
        Ok(Self::new(Duration::from_secs(ttl_secs), capacity))
    }

    /// Caching is on iff the TTL is non-zero.
    fn enabled(&self) -> bool {
        !self.ttl.is_zero()
    }

    /// Look up `key`. Returns the cached body bytes + their current age when a
    /// FRESH (non-expired) entry exists; drops the entry and returns `None` when
    /// it has expired or is absent (so an expired entry can never be served).
    pub(super) fn get(&self, key: &str) -> Option<(Arc<[u8]>, Duration)> {
        if !self.enabled() {
            return None;
        }
        let mut entries = self.entries.lock().expect("response-cache mutex poisoned");
        if let Some(entry) = entries.get(key) {
            let age = entry.stored.elapsed();
            if age < self.ttl {
                return Some((Arc::clone(&entry.body), age));
            }
        }
        entries.remove(key);
        None
    }

    /// Store `body` under `key`. Evicts expired entries, then the oldest-stored
    /// key, until `len <= capacity` — a hard memory bound.
    pub(super) fn put(&self, key: String, body: Arc<[u8]>) {
        if !self.enabled() {
            return;
        }
        let now = Instant::now();
        let mut entries = self.entries.lock().expect("response-cache mutex poisoned");
        entries.insert(key, CacheEntry { body, stored: now });
        if entries.len() > self.capacity {
            let ttl = self.ttl;
            entries.retain(|_, entry| now.duration_since(entry.stored) < ttl);
            while entries.len() > self.capacity {
                let Some(oldest) = entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.stored)
                    .map(|(key, _)| key.clone())
                else {
                    break;
                };
                entries.remove(&oldest);
            }
        }
    }
}

/// Parse a non-negative integer env var, returning `default` when unset and a
/// LOUD error when present-but-unparseable (never silently defaulted).
pub(super) fn parse_env_u64(name: &str, default: u64) -> Result<u64, String> {
    match std::env::var(name) {
        Err(_) => Ok(default),
        Ok(raw) => raw.trim().parse::<u64>().map_err(|error| {
            format!("{name} must be a non-negative integer ({error}); got {raw:?}")
        }),
    }
}

/// Build a `200 OK` JSON response from already-serialized `body` bytes, tagging
/// it with the standard cache-observability headers: `X-Cache: HIT|MISS`
/// (Varnish/CloudFront/Fastly convention) and `Age` (seconds since stored,
/// RFC 9111 §5.1). A HIT replays the EXACT cached bytes, so it is byte-identical
/// to the MISS that populated it.
pub(super) fn cached_json_response(
    body: Arc<[u8]>,
    cache_status: &'static str,
    age: Duration,
) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-cache", cache_status)
        .header(header::AGE, age.as_secs())
        .body(Body::from(body.to_vec()))
        .expect("static headers + byte body is always a valid response")
}

/// Serialize `body`, store it in `cache` under `key`, and return the `MISS`
/// response carrying the freshly-serialized bytes. A serialization failure is
/// logged in full and returned as a generic 500 (never cached).
pub(super) fn store_and_respond(cache: &ResponseCache, key: String, body: &Value) -> Response {
    match serde_json::to_vec(body) {
        Ok(bytes) => {
            let bytes: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
            cache.put(key, Arc::clone(&bytes));
            cached_json_response(bytes, "MISS", Duration::ZERO)
        }
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_CACHE_SERIALIZE_FAILED");
            ApiError::of(ErrorCode::Internal).into_response()
        }
    }
}
