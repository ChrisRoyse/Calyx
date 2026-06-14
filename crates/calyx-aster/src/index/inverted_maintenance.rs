use super::*;
use crate::mvcc::tombstone_value;

impl InvertedIndex {
    /// Stages all posting rows plus the updated stats row for one field value.
    /// The caller chooses the commit batch; this does not write to the vault.
    pub fn encode_put_entries(
        &self,
        field_val: &FieldValue,
        pk: &RecordKey,
        stats: InvertedStats,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let text = text_value(field_val)?;
        let (_counts, doc_len) = term_frequencies(text);
        if doc_len == 0 {
            return Ok(Vec::new());
        }
        let updated = updated_stats(stats, doc_len);
        let mut rows = self.encode_entries(field_val, pk, stats)?;
        rows.push((self.stats_key(), encode_stats(updated)));
        Ok(rows)
    }

    /// Stages tombstones for the posting keys produced by `field_val`.
    /// Stats are intentionally left for rebuild/self-heal because exact delete
    /// adjustment needs prior corpus totals that PH54 T05 owns.
    pub fn encode_delete_entries(
        &self,
        field_val: &FieldValue,
        pk: &RecordKey,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let text = text_value(field_val)?;
        Ok(tokenize(text)
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|term| (self.posting_key(&term, pk), tombstone_value()))
            .collect())
    }

    pub fn read_stats_at<C: Clock>(
        &self,
        vault: &AsterVault<C>,
        snapshot: Seq,
    ) -> Result<InvertedStats> {
        read_stats(vault, snapshot, self)
    }

    pub(crate) fn stats_after_put(
        &self,
        field_val: &FieldValue,
        stats: InvertedStats,
    ) -> Result<InvertedStats> {
        let text = text_value(field_val)?;
        let (_counts, doc_len) = term_frequencies(text);
        if doc_len == 0 {
            return Ok(stats);
        }
        Ok(updated_stats(stats, doc_len))
    }
}
