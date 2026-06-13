//! Bounded caches (PH56 — Stage S13).
//!
//! Every cached artifact in Calyx — lazy cross-terms, query plans, autotune
//! configs, kernel results — uses [`LruTtlCache`], so no cache in the system is
//! unbounded (A26): each has a hard byte cap, LRU eviction, and a per-entry TTL
//! driven by an injected [`Clock`](crate::Clock) (never `SystemTime::now()` in
//! logic, so tests are byte-deterministic).

pub mod lru_ttl;

pub use lru_ttl::{CALYX_CACHE_EVICTED, InsertResult, LruTtlCache};
