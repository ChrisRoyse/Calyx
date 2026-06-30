use calyx_search::SearchTraceEvent;

pub(super) fn emit_search_trace_event(event: SearchTraceEvent) {
    let slot = event
        .slot
        .map(|slot| slot.to_string())
        .unwrap_or_else(|| "-".to_string());
    let count = event
        .count
        .map(|count| count.to_string())
        .unwrap_or_else(|| "-".to_string());
    let detail = event.detail.unwrap_or_else(|| "-".to_string());
    eprintln!(
        "probe-matrix: search phase={} slot={} count={} elapsed_ms={} detail={}",
        event.phase, slot, count, event.elapsed_ms, detail
    );
}
