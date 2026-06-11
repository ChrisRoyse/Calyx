use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, Result};

use super::{HeadKind, head_key};
use crate::CALYX_ASTER_CF_UNAVAILABLE;

pub trait HeadStorage: Send + Sync {
    fn load_head(&self, kind: HeadKind) -> Result<Option<Vec<u8>>>;
    fn save_heads(&self, rows: Vec<(HeadKind, Vec<u8>)>) -> Result<()>;
    fn scan_heads(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;
}

pub struct AsterHeadStorage<'a, C>
where
    C: Clock,
{
    vault: &'a AsterVault<C>,
}

impl<'a, C> AsterHeadStorage<'a, C>
where
    C: Clock,
{
    pub const fn new(vault: &'a AsterVault<C>) -> Self {
        Self { vault }
    }
}

impl<C> HeadStorage for AsterHeadStorage<'_, C>
where
    C: Clock,
{
    fn load_head(&self, kind: HeadKind) -> Result<Option<Vec<u8>>> {
        self.vault
            .read_cf_at(
                self.vault.latest_seq(),
                ColumnFamily::AnnealHeads,
                &head_key(kind),
            )
            .map_err(|error| cf_unavailable("read anneal_heads CF", error))
    }

    fn save_heads(&self, rows: Vec<(HeadKind, Vec<u8>)>) -> Result<()> {
        self.vault
            .write_cf_batch(
                rows.into_iter()
                    .map(|(kind, value)| (ColumnFamily::AnnealHeads, head_key(kind), value)),
            )
            .map(|_| ())
            .map_err(|error| cf_unavailable("write anneal_heads CF", error))
    }

    fn scan_heads(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.vault
            .scan_cf_at(self.vault.latest_seq(), ColumnFamily::AnnealHeads)
            .map_err(|error| cf_unavailable("scan anneal_heads CF", error))
    }
}

fn cf_unavailable(context: &'static str, error: CalyxError) -> CalyxError {
    CalyxError {
        code: CALYX_ASTER_CF_UNAVAILABLE,
        message: format!("{context}: {}: {}", error.code, error.message),
        remediation: "repair the anneal_heads CF before retrying",
    }
}
