use super::*;

impl VersionedCfStore {
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
            let overlay = self.visible_table_entries(snapshot, cf, Some(range));
            let router = self.router.read().expect("mvcc router poisoned");
            if let Some(router) = router.as_ref() {
                return router.range_pages_until(
                    cf,
                    &range.start,
                    range.end.as_deref(),
                    limit,
                    overlay,
                    |entries| self.emit_entry_page(cf, entries, &mut on_page),
                );
            }
            return self.emit_entry_pages(cf, overlay, limit, &mut on_page);
        }
        let lower = Bound::Included((cf, range.start.clone()));
        let table = self.rows.read().expect("mvcc row table poisoned");
        let mut page = Vec::with_capacity(limit);
        for ((row_cf, key), versions) in table.range((lower, Bound::Unbounded)) {
            if *row_cf != cf {
                if *row_cf > cf {
                    break;
                }
                continue;
            }
            if !range.contains(key) {
                if range.end.as_ref().is_some_and(|end| key >= end) {
                    break;
                }
                continue;
            }
            if let Some(value) = visible_live_value(versions, snapshot.seq()) {
                self.ensure_unbarriered(cf, key).map_err(E::from)?;
                page.push((key.clone(), value));
                if page.len() == limit {
                    on_page(std::mem::take(&mut page))?;
                }
            }
        }
        if !page.is_empty() {
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

fn visible_live_value(versions: &[VersionedValue], seq: Seq) -> Option<Vec<u8>> {
    let version = visible_version(versions, seq)?;
    (!is_tombstone_value(&version.value)).then(|| version.value.clone())
}

fn visible_version(versions: &[VersionedValue], seq: Seq) -> Option<&VersionedValue> {
    versions.iter().rev().find(|version| version.seq <= seq)
}
