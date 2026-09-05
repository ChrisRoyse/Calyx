use super::*;

impl VersionedCfStore {
    /// Streams visible rows for one CF at the pinned sequence in bounded pages.
    pub fn scan_cf_pages_at<F, E>(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        limit: usize,
        clock: &dyn Clock,
        on_page: F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(Vec<(Vec<u8>, Vec<u8>)>) -> std::result::Result<(), E>,
        E: From<calyx_core::CalyxError>,
    {
        self.scan_cf_range_pages_at(snapshot, cf, &KeyRange::all(), limit, clock, on_page)
    }

    /// Streams visible rows in bounded pages without reopening SST readers per page.
    pub fn scan_cf_range_pages_at<F, E>(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        range: &KeyRange,
        limit: usize,
        clock: &dyn Clock,
        mut on_page: F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(Vec<(Vec<u8>, Vec<u8>)>) -> std::result::Result<(), E>,
        E: From<calyx_core::CalyxError>,
    {
        self.ensure_snapshot_live(snapshot, clock)
            .map_err(E::from)?;
        if limit == 0 {
            return Ok(());
        }
        if self.router_latest_readback.load(Ordering::Acquire) {
            // Detach one immutable SST merge cursor plus the bounded memtable
            // overlay, then release the router lock before invoking callbacks.
            // This streams each key/value exactly once; selecting the complete
            // key set and point-reading every row made large readbacks consume
            // O(rows) key memory and perform O(rows) independent SST lookups.
            self.ensure_router_latest_snapshot(snapshot)
                .map_err(E::from)?;
            let sources = {
                let router = self.router.read().expect("mvcc router poisoned");
                let Some(router) = router.as_ref() else {
                    return Ok(());
                };
                router.detached_range_page_sources(cf, &range.start, range.end.as_deref())
            }
            .map_err(E::from)?;
            let (level, overlay, value_crypto) = sources;
            level.range_pages_with_overlay(
                &range.start,
                range.end.as_deref(),
                None,
                limit,
                overlay,
                |entries| {
                    self.ensure_snapshot_live(snapshot, clock)
                        .map_err(E::from)?;
                    self.ensure_router_latest_snapshot(snapshot)
                        .map_err(E::from)?;
                    let mut page = Vec::with_capacity(entries.len());
                    for entry in entries {
                        self.ensure_unbarriered(cf, &entry.key).map_err(E::from)?;
                        let value = match &value_crypto {
                            Some(context) => crate::security::value_crypto::open_value(
                                context,
                                cf,
                                &entry.key,
                                &entry.value,
                            )
                            .map_err(E::from)?,
                            None => entry.value,
                        };
                        page.push((entry.key, value));
                    }
                    on_page(page)
                },
            )?;
            self.ensure_router_latest_snapshot(snapshot)
                .map_err(E::from)?;
            return Ok(());
        }
        let mut after_key = None::<Vec<u8>>;
        loop {
            let page = self
                .scan_cf_range_page_at(snapshot, cf, range, after_key.as_deref(), limit, clock)
                .map_err(E::from)?;
            let Some(last_key) = page.last().map(|(key, _)| key.clone()) else {
                break;
            };
            after_key = Some(last_key);
            on_page(page)?;
        }
        Ok(())
    }

    /// Streams the latest visible CF exactly once with a fresh bounded lease
    /// per page. The sequence is checked before and after every page, so a
    /// concurrent commit fails closed instead of mixing read views.
    pub(crate) fn scan_cf_pages_renewing_latest<F, E>(
        &self,
        seq: Seq,
        cf: ColumnFamily,
        limit: usize,
        clock: &dyn Clock,
        mut on_page: F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(Vec<(Vec<u8>, Vec<u8>)>) -> std::result::Result<(), E>,
        E: From<calyx_core::CalyxError>,
    {
        let ensure_latest = || -> std::result::Result<(), E> {
            let latest = self.current_seq();
            if latest != seq {
                return Err(E::from(calyx_core::CalyxError::stale_derived(format!(
                    "renewing {} scan requires unchanged latest sequence {seq}, observed {latest}",
                    cf.name()
                ))));
            }
            Ok(())
        };

        ensure_latest()?;
        if limit == 0 {
            return Ok(());
        }
        if self.router_latest_readback_enabled() {
            return self.scan_router_range_pages_renewing_latest(
                seq,
                cf,
                &KeyRange::all(),
                limit,
                clock,
                on_page,
            );
        }

        let range = KeyRange::all();
        let mut after_key = None::<Vec<u8>>;
        loop {
            ensure_latest()?;
            let snapshot = self.pin_snapshot_at(
                seq,
                Freshness::FreshDerived,
                clock,
                crate::gc::DEFAULT_READER_LEASE_MS,
            );
            let lease_id = snapshot.lease().id();
            let page = self
                .scan_cf_range_page_at(snapshot, cf, &range, after_key.as_deref(), limit, clock)
                .map_err(E::from);
            self.release_lease(lease_id);
            let page = page?;
            let Some(last_key) = page.last().map(|(key, _)| key.clone()) else {
                break;
            };
            after_key = Some(last_key);
            on_page(page)?;
            ensure_latest()?;
        }
        Ok(())
    }

    /// Streams one physical range from a latest-only recovered router while
    /// renewing the reader lease at every bounded page.
    ///
    /// The immutable SST merge cursor is opened once, so this remains O(rows)
    /// rather than reopening every SST for every page. Each page is validated
    /// under a fresh lease at the exact unchanged latest sequence, then the
    /// lease is released before the caller receives owned key/value bytes.
    pub(crate) fn scan_router_range_pages_renewing_latest<F, E>(
        &self,
        seq: Seq,
        cf: ColumnFamily,
        range: &KeyRange,
        limit: usize,
        clock: &dyn Clock,
        mut on_page: F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(Vec<(Vec<u8>, Vec<u8>)>) -> std::result::Result<(), E>,
        E: From<calyx_core::CalyxError>,
    {
        let ensure_latest = || -> std::result::Result<(), E> {
            let latest = self.current_seq();
            if latest != seq {
                return Err(E::from(calyx_core::CalyxError::stale_derived(format!(
                    "renewing {} range scan requires unchanged latest sequence {seq}, observed {latest}",
                    cf.name()
                ))));
            }
            Ok(())
        };

        ensure_latest()?;
        if limit == 0 {
            return Ok(());
        }
        if !self.router_latest_readback_enabled() {
            return Err(E::from(calyx_core::CalyxError::aster_corrupt_shard(
                "renewing physical range cursor requires a latest-only recovered CF router",
            )));
        }
        let overlay = {
            let snapshot = self.pin_snapshot_at(
                seq,
                Freshness::FreshDerived,
                clock,
                crate::gc::DEFAULT_READER_LEASE_MS,
            );
            let lease_id = snapshot.lease().id();
            let mut rows = BTreeMap::new();
            self.overlay_table_rows(snapshot, cf, Some(range), &mut rows);
            let live = self.ensure_snapshot_live(snapshot, clock).map_err(E::from);
            self.release_lease(lease_id);
            live?;
            rows.into_iter()
                .map(|(key, value)| crate::sst::SstEntry { key, value })
                .collect::<Vec<_>>()
        };
        let sources = {
            let router = self.router.read().expect("mvcc router poisoned");
            let router = router.as_ref().ok_or_else(|| {
                E::from(calyx_core::CalyxError::aster_corrupt_shard(
                    "latest-only recovered CF router is unavailable for renewing range scan",
                ))
            })?;
            router.detached_range_page_sources_renewing(
                cf,
                &range.start,
                range.end.as_deref(),
                overlay,
            )
        }
        .map_err(E::from)?;
        let (level, overlay, value_crypto) = sources;
        ensure_latest()?;

        level.range_pages_with_overlay(
            &range.start,
            range.end.as_deref(),
            None,
            limit,
            overlay,
            |entries| {
                ensure_latest()?;
                let snapshot = self.pin_snapshot_at(
                    seq,
                    Freshness::FreshDerived,
                    clock,
                    crate::mvcc::DEFAULT_READER_LEASE_MS,
                );
                let lease_id = snapshot.lease().id();
                let page_result = (|| -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
                    self.ensure_snapshot_live(snapshot, clock)?;
                    self.ensure_router_latest_snapshot(snapshot)?;
                    let mut page = Vec::with_capacity(entries.len());
                    for entry in entries {
                        self.ensure_unbarriered(cf, &entry.key)?;
                        let value = match &value_crypto {
                            Some(context) => crate::security::value_crypto::open_value(
                                context,
                                cf,
                                &entry.key,
                                &entry.value,
                            )?,
                            None => entry.value,
                        };
                        page.push((entry.key, value));
                    }
                    self.ensure_snapshot_live(snapshot, clock)?;
                    self.ensure_router_latest_snapshot(snapshot)?;
                    Ok(page)
                })();
                self.release_lease(lease_id);
                let page = page_result.map_err(E::from)?;
                on_page(page)?;
                ensure_latest()
            },
        )?;
        ensure_latest()
    }
}
