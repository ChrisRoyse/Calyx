use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Instant;

use calyx_core::{SlotId, SlotVector};
use calyx_registry::VaultPanelState;
use calyx_search::SearchTraceEvent;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::ProbeMatrixLog;
use super::resident;
use super::support::{accepted_hit_count, hex_lower};
use crate::error::CliResult;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProbeMatrixArtifactStatus {
    Ok,
    Refused,
    Incomplete,
}

impl ProbeMatrixArtifactStatus {
    pub(super) fn from_log(log: &ProbeMatrixLog) -> Self {
        if accepted_hit_count(log) > 0 && !log.productive.is_empty() {
            Self::Ok
        } else {
            Self::Refused
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ProbeMatrixDiagnostics {
    pub query_measurements: Vec<ProbeMatrixQueryMeasurement>,
    pub variant_guard_counts: Vec<ProbeMatrixVariantDiagnostic>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ProbeMatrixQueryMeasurement {
    pub query_text_sha256: String,
    pub measured_slot_count: usize,
    pub measure_call_count: usize,
    pub variant_use_count: usize,
    pub elapsed_ms: u128,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ProbeMatrixVariantDiagnostic {
    pub variant_id: usize,
    pub query_text_sha256: String,
    pub pre_guard_hit_count: Option<usize>,
    pub post_guard_hit_count: Option<usize>,
    pub guard_filtered_hit_count: Option<usize>,
}

pub(super) struct QueryVectorCache {
    allowed_slots: BTreeSet<SlotId>,
    entries: BTreeMap<String, CachedQueryVectors>,
}

struct CachedQueryVectors {
    query_text_sha256: String,
    vectors: Vec<(SlotId, SlotVector)>,
    elapsed_ms: u128,
    variant_use_count: usize,
}

impl QueryVectorCache {
    pub(super) fn new(allowed_slots: BTreeSet<SlotId>) -> Self {
        Self {
            allowed_slots,
            entries: BTreeMap::new(),
        }
    }

    pub(super) fn query_vectors<'a>(
        &'a mut self,
        state: &VaultPanelState,
        vault_dir: &Path,
        query: &str,
        resident_addr: Option<SocketAddr>,
    ) -> CliResult<(String, &'a [(SlotId, SlotVector)])> {
        if !self.entries.contains_key(query) {
            let started = Instant::now();
            let query_text_sha256 = sha256_text(query);
            eprintln!(
                "probe-matrix: query measurement cache_miss query_sha256={} selected_slots={} resident_addr={:?}",
                query_text_sha256,
                self.allowed_slots.len(),
                resident_addr
            );
            let vectors = match resident_addr {
                Some(addr) => resident::measure_query_vectors_via_resident(
                    state,
                    vault_dir,
                    query,
                    &self.allowed_slots,
                    addr,
                )?,
                None => calyx_search::engine::measure_query_vectors_with_slots(
                    state,
                    query,
                    Some(&self.allowed_slots),
                )?,
            };
            let elapsed_ms = started.elapsed().as_millis();
            eprintln!(
                "probe-matrix: query measurement cached query_sha256={} measured_slots={} elapsed_ms={}",
                query_text_sha256,
                vectors.len(),
                elapsed_ms
            );
            self.entries.insert(
                query.to_string(),
                CachedQueryVectors {
                    query_text_sha256,
                    vectors,
                    elapsed_ms,
                    variant_use_count: 0,
                },
            );
        }
        let entry = self
            .entries
            .get_mut(query)
            .expect("query vector cache entry inserted before readback");
        entry.variant_use_count += 1;
        eprintln!(
            "probe-matrix: query measurement cache_hit query_sha256={} use_count={} measured_slots={}",
            entry.query_text_sha256,
            entry.variant_use_count,
            entry.vectors.len()
        );
        Ok((entry.query_text_sha256.clone(), entry.vectors.as_slice()))
    }

    pub(super) fn diagnostics(&self) -> Vec<ProbeMatrixQueryMeasurement> {
        self.entries
            .values()
            .map(|entry| ProbeMatrixQueryMeasurement {
                query_text_sha256: entry.query_text_sha256.clone(),
                measured_slot_count: entry.vectors.len(),
                measure_call_count: 1,
                variant_use_count: entry.variant_use_count,
                elapsed_ms: entry.elapsed_ms,
            })
            .collect()
    }
}

pub(super) fn variant_guard_diagnostic(
    variant_id: usize,
    query_text_sha256: String,
    events: &[SearchTraceEvent],
) -> ProbeMatrixVariantDiagnostic {
    let pre = count_for_phase(events, "guard.in_region.start");
    let post = count_for_phase(events, "guard.in_region.done");
    ProbeMatrixVariantDiagnostic {
        variant_id,
        query_text_sha256,
        pre_guard_hit_count: pre,
        post_guard_hit_count: post,
        guard_filtered_hit_count: match (pre, post) {
            (Some(before), Some(after)) => Some(before.saturating_sub(after)),
            _ => None,
        },
    }
}

fn count_for_phase(events: &[SearchTraceEvent], phase: &str) -> Option<usize> {
    events
        .iter()
        .rev()
        .find(|event| event.phase == phase)
        .and_then(|event| event.count)
}

fn sha256_text(query: &str) -> String {
    hex_lower(&Sha256::digest(query.as_bytes()))
}
