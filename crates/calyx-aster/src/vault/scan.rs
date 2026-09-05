use super::*;

impl<C> AsterVault<C>
where
    C: Clock,
{
    /// Reads exact physical slot-CF rows at one already-pinned snapshot.
    ///
    /// Callers choose the batch size.  Unlike a range scan, this never
    /// materializes unrelated large embedding values and preserves input
    /// order (including explicit `None` entries for missing rows).
    pub fn read_slot_cf_batch_snapshot(
        &self,
        snapshot: Snapshot,
        slot: calyx_core::SlotId,
        ids: &[CxId],
    ) -> Result<Vec<Option<Vec<u8>>>> {
        let reads = ids
            .iter()
            .map(|id| crate::mvcc::CfRead::new(ColumnFamily::slot(slot), crate::cf::slot_key(*id)))
            .collect::<Vec<_>>();
        self.rows.read_batch(snapshot, &reads, &self.clock)
    }

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
        let snapshot = self.snapshot_handle(snapshot);
        self.rows
            .scan_cf_pages_at(snapshot.snapshot(), cf, limit, &self.clock, on_page)
    }

    /// Streams visible raw CF rows in one key range at `snapshot` using
    /// bounded value pages and a single pinned read view.
    pub fn scan_cf_range_pages_at<F, E>(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
        range: &KeyRange,
        limit: usize,
        on_page: F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(Vec<(Vec<u8>, Vec<u8>)>) -> std::result::Result<(), E>,
        E: From<calyx_core::CalyxError>,
    {
        let snapshot = self.snapshot_handle(snapshot);
        self.rows.scan_cf_range_pages_at(
            snapshot.snapshot(),
            cf,
            range,
            limit,
            &self.clock,
            on_page,
        )
    }

    /// Streams the latest visible raw CF rows with one bounded lease per page.
    ///
    /// This contract is for cold physical readbacks that can legitimately run
    /// longer than the normal reader-lease window. It does not lengthen or
    /// disable that window: visible keys are selected once, and every bounded
    /// value page receives a fresh lease at the same sequence. The lease is
    /// released before the callback runs. If any writer advances the vault,
    /// the scan fails closed instead of mixing sequences or renewing a stale
    /// view whose versions could have become reclaimable between pages.
    pub fn scan_cf_pages_at_renewing_latest<F, E>(
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
        self.scan_cf_range_pages_at_renewing_latest(snapshot, cf, &KeyRange::all(), limit, on_page)
    }

    /// Streams one latest physical CF range with a fresh bounded lease per
    /// page and fails closed if the latest sequence advances.
    pub fn scan_cf_range_pages_at_renewing_latest<F, E>(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
        range: &KeyRange,
        limit: usize,
        mut on_page: F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(Vec<(Vec<u8>, Vec<u8>)>) -> std::result::Result<(), E>,
        E: From<calyx_core::CalyxError>,
    {
        let ensure_latest = || -> std::result::Result<(), E> {
            let latest = self.latest_seq();
            if latest != snapshot {
                return Err(E::from(calyx_core::CalyxError::stale_derived(format!(
                    "renewing {} range scan requires unchanged latest sequence {snapshot}, observed {latest}",
                    cf.name()
                ))));
            }
            Ok(())
        };

        ensure_latest()?;
        if limit == 0 {
            return Ok(());
        }
        if self.rows.router_latest_readback_enabled() {
            return self.rows.scan_router_range_pages_renewing_latest(
                snapshot,
                cf,
                range,
                limit,
                &self.clock,
                on_page,
            );
        }
        let keys = self
            .scan_cf_range_keys_at(snapshot, cf, range)
            .map_err(E::from)?;
        ensure_latest()?;

        for page_keys in keys.chunks(limit) {
            ensure_latest()?;
            let page = {
                let pinned = self.snapshot_handle(snapshot);
                let reads = page_keys
                    .iter()
                    .cloned()
                    .map(|key| crate::mvcc::CfRead::new(cf, key))
                    .collect::<Vec<_>>();
                let values = self
                    .rows
                    .read_batch(pinned.snapshot(), &reads, &self.clock)
                    .map_err(E::from)?;
                page_keys
                    .iter()
                    .cloned()
                    .zip(values)
                    .map(|(key, value)| {
                        value.map(|value| (key.clone(), value)).ok_or_else(|| {
                            let key_prefix = key
                                .iter()
                                .take(8)
                                .map(|byte| format!("{byte:02x}"))
                                .collect::<String>();
                            E::from(calyx_core::CalyxError::aster_corrupt_shard(format!(
                                "visible {} key {} disappeared during renewing latest scan",
                                cf.name(),
                                key_prefix
                            )))
                        })
                    })
                    .collect::<std::result::Result<Vec<_>, E>>()?
            };
            on_page(page)?;
            ensure_latest()?;
        }
        Ok(())
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
