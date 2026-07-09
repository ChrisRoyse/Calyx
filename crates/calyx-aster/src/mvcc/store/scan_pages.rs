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
            self.ensure_router_latest_snapshot(snapshot)
                .map_err(E::from)?;
            let mut overlay = Some(self.visible_table_entries(snapshot, cf, Some(range)));
            let streamed = {
                let router = self.router.read().expect("mvcc router poisoned");
                if let Some(router) = router.as_ref() {
                    router.range_pages_until(
                        cf,
                        &range.start,
                        range.end.as_deref(),
                        limit,
                        overlay.take().expect("overlay not consumed"),
                        |entries| self.emit_entry_page(cf, entries, &mut on_page),
                    )?;
                    true
                } else {
                    false
                }
            };
            if streamed {
                return Ok(());
            }
            let overlay = overlay.expect("overlay retained when router is absent");
            return self.emit_entry_pages(cf, overlay, limit, &mut on_page);
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

    fn visible_table_entries(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        range: Option<&KeyRange>,
    ) -> Vec<SstEntry> {
        let table = self.rows.read().expect("mvcc row table poisoned");
        table
            .iter()
            .filter(|((row_cf, key), _)| {
                *row_cf == cf && range.is_none_or(|range| range.contains(key))
            })
            .filter_map(|((_, key), versions)| visible_entry(key, versions, snapshot.seq()))
            .collect()
    }

    fn emit_entry_pages<F, E>(
        &self,
        cf: ColumnFamily,
        entries: Vec<SstEntry>,
        limit: usize,
        on_page: &mut F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(Vec<(Vec<u8>, Vec<u8>)>) -> std::result::Result<(), E>,
        E: From<calyx_core::CalyxError>,
    {
        let mut rows = Vec::with_capacity(limit);
        for entry in entries
            .into_iter()
            .filter(|entry| !is_tombstone_value(&entry.value))
        {
            self.ensure_unbarriered(cf, &entry.key).map_err(E::from)?;
            rows.push((entry.key, entry.value));
            if rows.len() == limit {
                on_page(std::mem::take(&mut rows))?;
            }
        }
        if !rows.is_empty() {
            on_page(rows)?;
        }
        Ok(())
    }

    fn emit_entry_page<F, E>(
        &self,
        cf: ColumnFamily,
        entries: Vec<SstEntry>,
        on_page: &mut F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(Vec<(Vec<u8>, Vec<u8>)>) -> std::result::Result<(), E>,
        E: From<calyx_core::CalyxError>,
    {
        let mut rows = Vec::with_capacity(entries.len());
        for entry in entries {
            self.ensure_unbarriered(cf, &entry.key).map_err(E::from)?;
            rows.push((entry.key, entry.value));
        }
        if !rows.is_empty() {
            on_page(rows)?;
        }
        Ok(())
    }
}

fn visible_entry(key: &[u8], versions: &[VersionedValue], seq: Seq) -> Option<SstEntry> {
    let version = visible_version(versions, seq)?;
    Some(SstEntry {
        key: key.to_vec(),
        value: version.value.clone(),
    })
}

fn visible_version(versions: &[VersionedValue], seq: Seq) -> Option<&VersionedValue> {
    versions.iter().rev().find(|version| version.seq <= seq)
}
