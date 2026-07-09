use calyx_aster::cf::{ColumnFamily, base_key, recurrence_key};
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::{
    Occurrence, OccurrenceContext, StoredRecurrenceRow, encode_recurrence_row,
};
use calyx_aster::vault::encode;
use calyx_core::{CxId, VaultStore};

use crate::DomainId;
use crate::evidence::OracleEvidence;

use super::*;

#[test]
fn oracle_evidence_load_at_excludes_later_recurrence_rows() {
    let vault = vault();
    let panel = panel(&[1]);
    put_sufficiency(&vault, &panel, 1.0, 0.8);
    let cx_id = CxId::from_bytes([220; 16]);
    vault
        .write_cf(
            ColumnFamily::Base,
            base_key(cx_id),
            encode::encode_constellation_base(&constellation(cx_id, DOMAIN, ACTION))
                .expect("encode base"),
        )
        .expect("write base");
    let pinned = vault.snapshot();
    let occurrence = Occurrence {
        id: OccurrenceId(0),
        t_k: EpochSecs(1_000),
        context: OccurrenceContext::new(context(
            DOMAIN,
            ACTION,
            &Row::prediction("Pass", Some("ci_passed")),
        ))
        .expect("context"),
    };
    vault
        .write_cf(
            ColumnFamily::Recurrence,
            recurrence_key(cx_id, 0),
            encode_recurrence_row(&StoredRecurrenceRow::Occurrence(occurrence))
                .expect("encode recurrence"),
        )
        .expect("write late recurrence");

    let stale = OracleEvidence::load_at(&vault, &DomainId::from(DOMAIN), pinned).unwrap();
    assert_eq!(stale.stats.assay_scans, 1);
    assert_eq!(stale.stats.base_scans, 1);
    assert_eq!(stale.stats.domain_rows_scanned, 1);
    assert!(stale.observations.is_empty());

    let latest = OracleEvidence::load(&vault, &DomainId::from(DOMAIN)).unwrap();
    assert_eq!(latest.observations.len(), 1);
}
