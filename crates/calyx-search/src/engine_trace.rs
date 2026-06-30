use std::time::Instant;

use calyx_core::SlotId;

/// Structured search phase event for callers that need operational evidence.
///
/// The shared search library stays silent by default; CLI callers opt into
/// these events when they need durable stderr phase logs for FSV/debugging.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchTraceEvent {
    pub phase: &'static str,
    pub slot: Option<SlotId>,
    pub elapsed_ms: u128,
    pub count: Option<usize>,
    pub detail: Option<String>,
}

pub(crate) struct SearchTracer<'a> {
    started: Instant,
    sink: Option<&'a mut dyn FnMut(SearchTraceEvent)>,
}

impl<'a> SearchTracer<'a> {
    pub(crate) fn new(sink: Option<&'a mut dyn FnMut(SearchTraceEvent)>) -> Self {
        Self {
            started: Instant::now(),
            sink,
        }
    }

    pub(crate) fn emit(&mut self, phase: &'static str, slot: Option<SlotId>, count: Option<usize>) {
        self.emit_detail(phase, slot, count, None);
    }

    pub(crate) fn emit_detail(
        &mut self,
        phase: &'static str,
        slot: Option<SlotId>,
        count: Option<usize>,
        detail: Option<String>,
    ) {
        let Some(sink) = self.sink.as_deref_mut() else {
            return;
        };
        sink(SearchTraceEvent {
            phase,
            slot,
            elapsed_ms: self.started.elapsed().as_millis(),
            count,
            detail,
        });
    }
}
