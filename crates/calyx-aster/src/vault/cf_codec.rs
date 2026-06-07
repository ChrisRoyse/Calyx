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
        16..=63 => ColumnFamily::slot(SlotId::new((tag - 16) as u16)),
        64..=111 => ColumnFamily::slot_raw(SlotId::new((tag - 64) as u16)),
        _ => return Err(CalyxError::aster_corrupt_shard(format!("unknown CF tag {tag}"))),
    })
}
