use super::{ColumnFamily, router::CfRouter};
use crate::security::value_crypto::{SharedVaultContext, seal_value};
use crate::sst::SstEntry;
use crate::sst::level::SstLevel;
use calyx_core::CalyxError;

impl CfRouter {
    pub(crate) fn detached_range_page_sources_renewing(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: Option<&[u8]>,
        overlay: Vec<SstEntry>,
    ) -> calyx_core::Result<(
        SstLevel,
        Vec<SstEntry>,
        Option<crate::security::value_crypto::SharedVaultContext>,
    )> {
        let (level, mut overlay) = self.range_page_sources(cf, start, end, overlay);
        overlay.sort_by(|left, right| left.key.cmp(&right.key));
        let mut unique = Vec::<SstEntry>::with_capacity(overlay.len());
        for entry in overlay {
            if let Some(previous) = unique.last()
                && previous.key == entry.key
            {
                if previous.value != entry.value {
                    return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
                        "router and MVCC table disagree for {} key {}",
                        cf.name(),
                        entry
                            .key
                            .iter()
                            .take(8)
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>()
                    )));
                }
                continue;
            }
            unique.push(entry);
        }
        let mut overlay = unique;
        let value_crypto = self.value_crypto_context();
        if let Some(context) = &value_crypto {
            for entry in &mut overlay {
                entry.value = crate::security::value_crypto::seal_value(
                    context,
                    cf,
                    &entry.key,
                    &entry.value,
                )?;
            }
        }
        Ok((level, overlay, value_crypto))
    }

    pub(crate) fn detached_range_page_sources(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: Option<&[u8]>,
    ) -> calyx_core::Result<(SstLevel, Vec<SstEntry>, Option<SharedVaultContext>)> {
        let (level, mut overlay) = self.range_page_sources(cf, start, end, Vec::new());
        if let Some(context) = &self.value_crypto {
            for entry in &mut overlay {
                entry.value = seal_value(context, cf, &entry.key, &entry.value)?;
            }
        }
        Ok((level, overlay, self.value_crypto.clone()))
    }

    pub(crate) fn range_page_sources(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: Option<&[u8]>,
        mut overlay: Vec<SstEntry>,
    ) -> (SstLevel, Vec<SstEntry>) {
        if let Some(table) = self.memtables.get(&cf) {
            overlay.extend(
                table
                    .iter()
                    .filter(|(key, _)| key.as_slice() >= start)
                    .filter(|(key, _)| end.is_none_or(|end| key.as_slice() < end))
                    .map(|(key, value)| SstEntry { key, value }),
            );
        }
        (self.levels.get(&cf).cloned().unwrap_or_default(), overlay)
    }

    pub fn range_pages_until<F, E>(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: Option<&[u8]>,
        limit: usize,
        overlay: Vec<SstEntry>,
        mut on_page: F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(Vec<SstEntry>) -> std::result::Result<(), E>,
        E: From<CalyxError>,
    {
        if limit == 0 {
            return Ok(());
        }
        let (level, overlay) = self.range_page_sources(cf, start, end, overlay);
        level.range_pages_with_overlay(start, end, None, limit, overlay, |entries| {
            on_page(self.open_entries(cf, entries).map_err(E::from)?)
        })
    }
}
