use calyx_core::{CxId, SlotId};
use calyx_sextant::IndexSearchHit;
use calyx_sextant::index::partitioned::cx;

pub(super) fn to_index_hits(rows: Vec<(u64, f32)>) -> Vec<IndexSearchHit> {
    rows.into_iter()
        .enumerate()
        .map(|(idx, (id, score))| IndexSearchHit {
            cx_id: cx(id),
            score,
            rank: idx + 1,
        })
        .collect()
}

pub(super) fn hit_ids(hits: &[IndexSearchHit], k: usize) -> Vec<u64> {
    hits.iter().take(k).map(|hit| low_u64(hit.cx_id)).collect()
}

pub(super) fn fused_hit_ids(hits: &[calyx_sextant::Hit], k: usize) -> Vec<u64> {
    hits.iter().take(k).map(|hit| low_u64(hit.cx_id)).collect()
}

pub(super) fn slot_id(value: u16) -> SlotId {
    SlotId::new(value)
}

pub(super) fn low_u64(cx_id: CxId) -> u64 {
    let bytes = cx_id.as_bytes();
    u64::from_be_bytes(bytes[8..16].try_into().expect("CxId is 16 bytes"))
}
