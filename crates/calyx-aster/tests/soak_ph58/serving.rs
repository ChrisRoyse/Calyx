use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::Clock;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug)]
pub(crate) struct LiveReadbackCount {
    pub(crate) visible: usize,
    pub(crate) missing: usize,
}

impl LiveReadbackCount {
    pub(crate) fn to_json(self) -> Value {
        json!({
            "visible": self.visible,
            "missing": self.missing,
        })
    }
}

pub(crate) fn live_base_readback_count<C: Clock>(
    vault: &AsterVault<C>,
    start: u64,
    end: u64,
) -> LiveReadbackCount {
    let snapshot = vault.latest_seq();
    let mut visible = 0usize;
    let mut missing = 0usize;
    for id in start..end {
        let key = format!("key-{id:05}");
        match vault
            .read_cf_at(snapshot, ColumnFamily::Base, key.as_bytes())
            .expect("serving readback")
        {
            Some(_) => visible += 1,
            None => missing += 1,
        }
    }
    LiveReadbackCount { visible, missing }
}
