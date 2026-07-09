use super::{ColumnFamily, router::CfRouter};
use crate::sst::SstEntry;
use crate::sst::level::SstLevel;
use calyx_core::CalyxError;

impl CfRouter {
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
        on_page: F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(Vec<SstEntry>) -> std::result::Result<(), E>,
        E: From<CalyxError>,
    {
        if limit == 0 {
            return Ok(());
        }
        let (level, overlay) = self.range_page_sources(cf, start, end, overlay);
        level.range_pages_with_overlay(start, end, None, limit, overlay, on_page)
    }
}
