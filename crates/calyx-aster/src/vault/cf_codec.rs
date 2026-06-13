use crate::cf::{ColumnFamily, SlotFamilyKind};
use calyx_core::{CalyxError, Result, SlotId};

pub(super) fn cf_tag(cf: ColumnFamily) -> u8 {
    match cf {
        ColumnFamily::Base => 0,
        ColumnFamily::Anchors => 1,
        ColumnFamily::Ledger => 2,
        ColumnFamily::XTerm => 3,
        ColumnFamily::Scalars => 4,
        ColumnFamily::Online => 5,
        ColumnFamily::Assay => 6,
        ColumnFamily::Recurrence => 7,
        ColumnFamily::TemporalXTerm => 8,
        ColumnFamily::AnnealRollback => 9,
        ColumnFamily::AnnealHealth => 10,
        ColumnFamily::AnnealChecksums => 11,
        ColumnFamily::Graph => 12,
        ColumnFamily::AnnealMistakes => 13,
        ColumnFamily::AnnealReplay => 14,
        ColumnFamily::AnnealHeads => 15,
        ColumnFamily::AnnealBandit => 112,
        ColumnFamily::AnnealSoak => 113,
        ColumnFamily::AnnealReport => 114,
        ColumnFamily::AnnealGrowth => 115,
        ColumnFamily::TimeIndex => 116,
        ColumnFamily::Slot { slot, kind } => {
            let base = match kind {
                SlotFamilyKind::Quantized => 16,
                SlotFamilyKind::Raw => 64,
            };
            base + slot.get() as u8
        }
    }
}

pub(super) fn decode_cf(tag: u8) -> Result<ColumnFamily> {
    Ok(match tag {
        0 => ColumnFamily::Base,
        1 => ColumnFamily::Anchors,
        2 => ColumnFamily::Ledger,
        3 => ColumnFamily::XTerm,
        4 => ColumnFamily::Scalars,
        5 => ColumnFamily::Online,
        6 => ColumnFamily::Assay,
        7 => ColumnFamily::Recurrence,
        8 => ColumnFamily::TemporalXTerm,
        9 => ColumnFamily::AnnealRollback,
        10 => ColumnFamily::AnnealHealth,
        11 => ColumnFamily::AnnealChecksums,
        12 => ColumnFamily::Graph,
        13 => ColumnFamily::AnnealMistakes,
        14 => ColumnFamily::AnnealReplay,
        15 => ColumnFamily::AnnealHeads,
        112 => ColumnFamily::AnnealBandit,
        113 => ColumnFamily::AnnealSoak,
        114 => ColumnFamily::AnnealReport,
        115 => ColumnFamily::AnnealGrowth,
        116 => ColumnFamily::TimeIndex,
        16..=63 => ColumnFamily::slot(SlotId::new((tag - 16) as u16)),
        64..=111 => ColumnFamily::slot_raw(SlotId::new((tag - 64) as u16)),
        _ => {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "unknown CF tag {tag}"
            )));
        }
    })
}
