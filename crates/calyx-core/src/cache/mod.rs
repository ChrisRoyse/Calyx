//! Bounded caches (PH56 — Stage S13).
//!
//! [`LruTtlCache`] is the canonical bounded-cache primitive for callers that
//! opt into A26: it has a hard byte cap, LRU eviction, and a per-entry TTL
//! driven by an injected [`Clock`](crate::Clock) (never `SystemTime::now()` in
//! logic, so tests are byte-deterministic). Not every cache in the workspace is
//! wired through this module yet; callers that use ad-hoc maps are outside
//! these cache counters until migrated.

pub mod lru_ttl;

pub use lru_ttl::{CALYX_CACHE_EVICTED, InsertResult, LruTtlCache};
