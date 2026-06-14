//! Association-native Aster column families and key codecs.

mod family;
mod key;
mod router;

#[cfg(test)]
mod router_tests;

pub use family::{ColumnFamily, SlotFamilyKind};
pub use key::{
    KeyRange, OnlineKeyKind, ScalarId, XTermKind, anchor_key, anchor_prefix_range, base_key,
    cx_id_from_full_hash, cx_prefix_range, full_content_hash, ledger_key, ledger_range, online_key,
    prefix_range, recurrence_key, recurrence_prefix_range, scalar_key, scalar_prefix_range,
    slot_key, temporal_xterm_key, temporal_xterm_prefix_range, verify_cx_hash_prefix, xterm_key,
    xterm_prefix_range,
};
pub use router::CfRouter;

#[cfg(test)]
mod tests;
