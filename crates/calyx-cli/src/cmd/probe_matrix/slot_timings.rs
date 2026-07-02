//! Per-slot search timing extracted from search trace events into the matrix
//! artifact (issue #1102): FSV asserts performance budgets from JSON instead
//! of scraping stderr.

use calyx_search::SearchTraceEvent;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ProbeMatrixSlotSearchDiagnostic {
    pub slot: u16,
    pub hit_count: usize,
    /// Wall-clock spent scoring this slot when the slot-result cache missed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u128>,
    /// The original scoring cost replayed by a slot-result cache hit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_elapsed_ms: Option<u128>,
    pub cache_hit: bool,
}

pub(super) fn slot_search_diagnostics(
    events: &[SearchTraceEvent],
) -> Vec<ProbeMatrixSlotSearchDiagnostic> {
    let mut out = Vec::new();
    for event in events {
        let Some(slot) = event.slot else {
            continue;
        };
        match event.phase {
            "search_slot.done" => out.push(ProbeMatrixSlotSearchDiagnostic {
                slot: slot.get(),
                hit_count: event.count.unwrap_or(0),
                elapsed_ms: detail_u128(event.detail.as_deref(), "slot_elapsed_ms"),
                source_elapsed_ms: None,
                cache_hit: false,
            }),
            "search_slot.cache_hit" => out.push(ProbeMatrixSlotSearchDiagnostic {
                slot: slot.get(),
                hit_count: event.count.unwrap_or(0),
                elapsed_ms: None,
                source_elapsed_ms: detail_u128(event.detail.as_deref(), "source_slot_elapsed_ms"),
                cache_hit: true,
            }),
            _ => {}
        }
    }
    out
}

fn detail_u128(detail: Option<&str>, field: &str) -> Option<u128> {
    detail?
        .split_whitespace()
        .find_map(|part| part.strip_prefix(field)?.strip_prefix('='))
        .and_then(|value| value.parse::<u128>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        phase: &'static str,
        slot: Option<calyx_core::SlotId>,
        count: Option<usize>,
        detail: Option<&str>,
    ) -> SearchTraceEvent {
        SearchTraceEvent {
            phase,
            slot,
            elapsed_ms: 1,
            count,
            detail: detail.map(str::to_string),
        }
    }

    #[test]
    fn extracts_miss_and_cache_hit_slot_timings() {
        let events = vec![
            event("search_slots.cache.miss", None, Some(13), None),
            event(
                "search_slot.done",
                Some(calyx_core::SlotId::new(22)),
                Some(10),
                Some("slot_elapsed_ms=26683"),
            ),
            event(
                "search_slot.cache_hit",
                Some(calyx_core::SlotId::new(13)),
                Some(4),
                Some("key_sha256=k source_slot_elapsed_ms=6579"),
            ),
            event("search.done", None, Some(5), None),
        ];
        let rows = slot_search_diagnostics(&events);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].slot, 22);
        assert_eq!(rows[0].hit_count, 10);
        assert_eq!(rows[0].elapsed_ms, Some(26683));
        assert!(!rows[0].cache_hit);
        assert_eq!(rows[1].slot, 13);
        assert_eq!(rows[1].source_elapsed_ms, Some(6579));
        assert!(rows[1].cache_hit);
    }

    #[test]
    fn missing_detail_field_yields_no_elapsed_instead_of_garbage() {
        let events = vec![event(
            "search_slot.done",
            Some(calyx_core::SlotId::new(22)),
            Some(3),
            Some("noise"),
        )];
        let rows = slot_search_diagnostics(&events);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].elapsed_ms, None);
    }
}
