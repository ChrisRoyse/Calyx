#![no_main]

use calyx_sextant::{Query, QueryPlanner};
use libfuzzer_sys::fuzz_target;

const MAX_INPUT: usize = 1 << 20;

fuzz_target!(|data: &[u8]| {
    let data = bounded(data);
    if let Ok(query) = serde_json::from_slice::<Query>(data) {
        let _ = query.validate();
        let index_size = data.first().map_or(0, |byte| usize::from(*byte));
        let _ = QueryPlanner::default().plan(query, index_size);
    }
});

fn bounded(data: &[u8]) -> &[u8] {
    &data[..data.len().min(MAX_INPUT)]
}
