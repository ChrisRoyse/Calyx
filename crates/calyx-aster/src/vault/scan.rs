use super::*;

impl<C> AsterVault<C>
where
    C: Clock,
{
    /// Streams visible raw CF rows at `snapshot` in bounded pages.
    pub fn scan_cf_pages_at<F, E>(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
        limit: usize,
        on_page: F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(Vec<(Vec<u8>, Vec<u8>)>) -> std::result::Result<(), E>,
        E: From<calyx_core::CalyxError>,
    {
        self.rows.scan_cf_pages_at(
            self.snapshot_handle(snapshot),
            cf,
            limit,
            &self.clock,
            on_page,
        )
    }

    /// Streams visible raw CF rows using an already-pinned snapshot lease.
    pub fn scan_cf_pages_snapshot<F, E>(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        limit: usize,
        on_page: F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(Vec<(Vec<u8>, Vec<u8>)>) -> std::result::Result<(), E>,
        E: From<calyx_core::CalyxError>,
    {
        self.rows
            .scan_cf_pages_at(snapshot, cf, limit, &self.clock, on_page)
    }

    /// Scans at most `limit` visible raw CF rows using an already-pinned snapshot lease.
    pub fn scan_cf_range_page_snapshot(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        range: &KeyRange,
        after_key: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.rows
            .scan_cf_range_page_at(snapshot, cf, range, after_key, limit, &self.clock)
    }

    /// Streams visible raw CF rows in bounded pages using an already-pinned snapshot lease.
    pub fn scan_cf_range_pages_snapshot<F, E>(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        range: &KeyRange,
        limit: usize,
        on_page: F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(Vec<(Vec<u8>, Vec<u8>)>) -> std::result::Result<(), E>,
        E: From<calyx_core::CalyxError>,
    {
        self.rows
            .scan_cf_range_pages_at(snapshot, cf, range, limit, &self.clock, on_page)
    }
}
