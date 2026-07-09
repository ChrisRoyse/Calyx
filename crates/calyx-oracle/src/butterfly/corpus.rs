use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, encode};
use calyx_core::{Clock, Constellation, VaultStore};

use crate::evidence_error;
use crate::{
    DomainId, ORACLE_DOMAIN_METADATA_KEY, ORACLE_FALLBACK_DOMAIN_METADATA_KEY, OracleError,
};

pub(super) struct DomainCorpus {
    rows: Vec<Constellation>,
}

impl DomainCorpus {
    pub(super) fn load<C>(
        vault: &AsterVault<C>,
        domain: &DomainId,
    ) -> Result<(Self, u64), OracleError>
    where
        C: Clock,
    {
        let rows = vault
            .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
            .map_err(|_| evidence_error::storage_read(domain, "scan base corpus"))?;
        let scanned = rows.len() as u64;
        let mut domain_rows = Vec::new();
        for (_, bytes) in rows {
            let cx = encode::decode_constellation_base(&bytes)
                .map_err(|_| evidence_error::corrupt(domain, "base constellation"))?;
            if matches_domain(&cx, domain) {
                domain_rows.push(cx);
            }
        }
        Ok((Self { rows: domain_rows }, scanned))
    }

    pub(super) fn rows(&self) -> &[Constellation] {
        &self.rows
    }
}

fn matches_domain(cx: &Constellation, domain: &DomainId) -> bool {
    cx.metadata_value(ORACLE_DOMAIN_METADATA_KEY) == Some(domain.as_str())
        || cx.metadata_value(ORACLE_FALLBACK_DOMAIN_METADATA_KEY) == Some(domain.as_str())
}
