use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, Result};

pub trait OperatorProposalStorage: Send + Sync {
    fn save_operator_proposal(&self, key: Vec<u8>, value: Vec<u8>) -> Result<()>;
    fn load_operator_proposal(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    fn scan_operator_proposals(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;
}

pub struct AsterOperatorProposalStorage<'a, C>
where
    C: Clock,
{
    vault: &'a AsterVault<C>,
}

impl<'a, C> AsterOperatorProposalStorage<'a, C>
where
    C: Clock,
{
    pub const fn new(vault: &'a AsterVault<C>) -> Self {
        Self { vault }
    }
}

impl<C> OperatorProposalStorage for AsterOperatorProposalStorage<'_, C>
where
    C: Clock,
{
    fn save_operator_proposal(&self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        self.vault
            .write_cf(ColumnFamily::AnnealOperators, key, value)?;
        Ok(())
    }

    fn load_operator_proposal(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.vault
            .read_cf_at(self.vault.latest_seq(), ColumnFamily::AnnealOperators, key)
    }

    fn scan_operator_proposals(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.vault
            .scan_cf_at(self.vault.latest_seq(), ColumnFamily::AnnealOperators)
    }
}
