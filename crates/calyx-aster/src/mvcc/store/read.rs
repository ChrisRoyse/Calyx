use super::*;

mod latest;

impl VersionedCfStore {
    /// Reads one CF/key at the pinned sequence.
    pub fn read_at(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        key: &[u8],
        clock: &dyn Clock,
    ) -> Result<Option<Vec<u8>>> {
        self.ensure_snapshot_live(snapshot, clock)?;
        self.ensure_unbarriered(cf, key)?;
        {
            let table = self.rows.read().expect("mvcc row table poisoned");
            if let Some(value) = table
                .get(&cf)
                .and_then(|rows| rows.get(key))
                .and_then(|versions| visible_value_state(versions, snapshot.seq()))
            {
                return Ok(value.into_option());
            }
        }
        self.router_latest_value(snapshot, cf, key)
    }

    /// Returns the visible version sequence for one CF/key at the pinned sequence.
    pub fn seq_for_key_at(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        key: &[u8],
        clock: &dyn Clock,
    ) -> Result<Option<Seq>> {
        self.ensure_snapshot_live(snapshot, clock)?;
        self.ensure_unbarriered(cf, key)?;
        let table = self.rows.read().expect("mvcc row table poisoned");
        let seq = table
            .get(&cf)
            .and_then(|rows| rows.get(key))
            .and_then(|versions| visible_version(versions, snapshot.seq()))
            .map(|version| version.seq);
        if seq.is_some() || !self.router_latest_readback.load(Ordering::Acquire) {
            return Ok(seq);
        }
        self.ensure_router_latest_snapshot(snapshot)?;
        Err(latest_only_error(format!(
            "row sequence for {} key {} is unavailable because this vault was opened in latest-only recovery mode",
            cf.name(),
            hex_prefix(key)
        )))
    }

    /// Resolves all requested CF/key rows at the same pinned sequence.
    pub fn read_batch(
        &self,
        snapshot: Snapshot,
        reads: &[CfRead],
        clock: &dyn Clock,
    ) -> Result<Vec<Option<Vec<u8>>>> {
        self.ensure_snapshot_live(snapshot, clock)?;
        if reads.is_empty() {
            return Ok(Vec::new());
        }

        // Hold one shared barrier generation across table and router
        // resolution. An installer needs the write guard, so no requested key
        // can become blocked halfway through this logical batch.
        let barriers = self
            .read_barriers
            .read()
            .expect("mvcc read barriers poisoned");
        #[cfg(test)]
        self.batch_barrier_phases.fetch_add(1, Ordering::Relaxed);
        for read in reads {
            if let Some(error) = first_blocking(&barriers, read.cf, &read.key) {
                return Err(error);
            }
        }

        let mut values = vec![None; reads.len()];
        let mut router_misses = Vec::new();
        {
            let table = self.rows.read().expect("mvcc row table poisoned");
            #[cfg(test)]
            self.batch_row_phases.fetch_add(1, Ordering::Relaxed);
            for (index, read) in reads.iter().enumerate() {
                let visible = table
                    .get(&read.cf)
                    .and_then(|rows| rows.get(read.key.as_slice()))
                    .and_then(|versions| visible_value_state(versions, snapshot.seq()));
                match visible {
                    Some(VisibleValue::Live(value)) => values[index] = Some(value),
                    Some(VisibleValue::Tombstone) => {}
                    None => router_misses.push(index),
                }
            }
        }

        if router_misses.is_empty() || !self.router_latest_readback.load(Ordering::Acquire) {
            return Ok(values);
        }

        self.ensure_router_latest_snapshot(snapshot)?;
        let router = self.router.read().expect("mvcc router poisoned");
        #[cfg(test)]
        self.batch_router_phases.fetch_add(1, Ordering::Relaxed);
        if let Some(router) = router.as_ref() {
            for index in router_misses {
                let read = &reads[index];
                values[index] = router
                    .get(read.cf, &read.key)?
                    .filter(|value| !is_tombstone_value(value));
            }
        }
        Ok(values)
    }

    /// Scans visible rows for one CF at the pinned sequence, ordered by key.
    pub fn scan_cf_at(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        clock: &dyn Clock,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.ensure_snapshot_live(snapshot, clock)?;
        let mut rows = self.router_latest_rows(snapshot, cf, None)?;
        self.overlay_table_rows(snapshot, cf, None, &mut rows);
        for key in rows.keys() {
            self.ensure_unbarriered(cf, key)?;
        }
        Ok(rows.into_iter().collect())
    }

    /// Scans visible rows for one CF and key range at the pinned sequence.
    pub fn scan_cf_range_at(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        range: &KeyRange,
        clock: &dyn Clock,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.ensure_snapshot_live(snapshot, clock)?;
        let mut rows = self.router_latest_rows(snapshot, cf, Some(range))?;
        self.overlay_table_rows(snapshot, cf, Some(range), &mut rows);
        for key in rows.keys() {
            self.ensure_unbarriered(cf, key)?;
        }
        Ok(rows.into_iter().collect())
    }

    /// Scans visible row keys for one CF and key range at the pinned sequence.
    pub fn scan_cf_range_keys_at(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        range: &KeyRange,
        clock: &dyn Clock,
    ) -> Result<Vec<Vec<u8>>> {
        self.ensure_snapshot_live(snapshot, clock)?;
        let mut keys = self.router_latest_keys(snapshot, cf, range)?;
        self.overlay_table_keys(snapshot, cf, range, &mut keys);
        for key in keys.keys() {
            self.ensure_unbarriered(cf, key)?;
        }
        Ok(keys.into_keys().collect())
    }

    /// Scans at most `limit` visible rows in a range after `after_key`.
    pub fn scan_cf_range_page_at(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        range: &KeyRange,
        after_key: Option<&[u8]>,
        limit: usize,
        clock: &dyn Clock,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.ensure_snapshot_live(snapshot, clock)?;
        if limit == 0 {
            return Ok(Vec::new());
        }
        if self.router_latest_readback.load(Ordering::Acquire) {
            // The recovered router is the sole latest-readback source of truth
            // and live commits route every row into its memtables before making
            // the sequence visible. Use the SST merge cursor directly: the old
            // path materialized every visible key in the range for *each* page,
            // turning a bounded physical readback into O(rows * pages) work.
            self.ensure_router_latest_snapshot(snapshot)?;
            let router = self.router.read().expect("mvcc router poisoned");
            let Some(router) = router.as_ref() else {
                return Ok(Vec::new());
            };
            let page = router.range_page_until(
                cf,
                &range.start,
                range.end.as_deref(),
                after_key,
                limit,
            )?;
            for entry in &page {
                self.ensure_unbarriered(cf, &entry.key)?;
            }
            return Ok(page
                .into_iter()
                .map(|entry| (entry.key, entry.value))
                .collect());
        }
        let lower = if let Some(after_key) = after_key {
            Bound::Excluded(after_key)
        } else {
            Bound::Included(range.start.as_slice())
        };
        let table = self.rows.read().expect("mvcc row table poisoned");
        let mut rows = Vec::with_capacity(limit);
        let Some(cf_rows) = table.get(&cf) else {
            return Ok(rows);
        };
        for (key, versions) in cf_rows.range::<[u8], _>((lower, Bound::Unbounded)) {
            if !range.contains(key) {
                if range.end.as_ref().is_some_and(|end| key >= end) {
                    break;
                }
                continue;
            }
            if let Some(value) = visible_value(versions, snapshot.seq()) {
                self.ensure_unbarriered(cf, key)?;
                rows.push((key.clone(), value));
                if rows.len() == limit {
                    break;
                }
            }
        }
        Ok(rows)
    }

    /// Returns the greatest visible row in `[start, upper]` at one pinned snapshot.
    pub fn predecessor_cf_at(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        start: &[u8],
        upper: &[u8],
        clock: &dyn Clock,
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        self.ensure_snapshot_live(snapshot, clock)?;
        let router = if self.router_latest_readback.load(Ordering::Acquire) {
            self.ensure_router_latest_snapshot(snapshot)?;
            self.router.read().expect("mvcc router poisoned")
        } else {
            self.router.read().expect("mvcc router poisoned")
        };
        let mut upper = upper.to_vec();
        let mut inclusive = true;
        loop {
            let table_candidate =
                self.table_predecessor_state(snapshot, cf, start, &upper, inclusive);
            let router_candidate = if self.router_latest_readback.load(Ordering::Acquire) {
                router
                    .as_ref()
                    .map(|router| router.predecessor(cf, start, &upper, inclusive))
                    .transpose()?
                    .flatten()
            } else {
                None
            };
            match (table_candidate, router_candidate) {
                (None, None) => return Ok(None),
                (None, Some(row)) => {
                    self.ensure_unbarriered(cf, &row.key)?;
                    return Ok(Some((row.key, row.value)));
                }
                (Some((key, VisibleValue::Live(value))), None) => {
                    self.ensure_unbarriered(cf, &key)?;
                    return Ok(Some((key, value)));
                }
                (Some((key, VisibleValue::Tombstone)), None) => {
                    upper = key;
                    inclusive = false;
                }
                (Some((key, state)), Some(router_row)) => {
                    if router_row.key > key {
                        self.ensure_unbarriered(cf, &router_row.key)?;
                        return Ok(Some((router_row.key, router_row.value)));
                    }
                    match state {
                        VisibleValue::Live(value) => {
                            self.ensure_unbarriered(cf, &key)?;
                            return Ok(Some((key, value)));
                        }
                        VisibleValue::Tombstone => {
                            upper = key;
                            inclusive = false;
                        }
                    }
                }
            }
        }
    }

    fn table_predecessor_state(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        start: &[u8],
        upper: &[u8],
        inclusive: bool,
    ) -> Option<(Vec<u8>, VisibleValue)> {
        let lower = Bound::Included(start);
        let upper = if inclusive {
            Bound::Included(upper)
        } else {
            Bound::Excluded(upper)
        };
        let table = self.rows.read().expect("mvcc row table poisoned");
        table
            .get(&cf)?
            .range::<[u8], _>((lower, upper))
            .rev()
            .find_map(|(key, versions)| {
                visible_value_state(versions, snapshot.seq()).map(|state| (key.clone(), state))
            })
    }

    pub(super) fn ensure_unbarriered(&self, cf: ColumnFamily, key: &[u8]) -> Result<()> {
        let barriers = self
            .read_barriers
            .read()
            .expect("mvcc read barriers poisoned");
        if let Some(error) = first_blocking(&barriers, cf, key) {
            return Err(error);
        }
        Ok(())
    }
}

fn visible_value(versions: &[VersionedValue], seq: Seq) -> Option<Vec<u8>> {
    visible_value_state(versions, seq).and_then(VisibleValue::into_option)
}

enum VisibleValue {
    Live(Vec<u8>),
    Tombstone,
}

impl VisibleValue {
    fn into_option(self) -> Option<Vec<u8>> {
        match self {
            Self::Live(value) => Some(value),
            Self::Tombstone => None,
        }
    }
}

fn visible_value_state(versions: &[VersionedValue], seq: Seq) -> Option<VisibleValue> {
    visible_version(versions, seq).map(|version| {
        if is_tombstone_value(&version.value) {
            VisibleValue::Tombstone
        } else {
            VisibleValue::Live(version.value.clone())
        }
    })
}

fn visible_version(versions: &[VersionedValue], seq: Seq) -> Option<&VersionedValue> {
    versions.iter().rev().find(|version| version.seq <= seq)
}

fn latest_only_error(message: impl Into<String>) -> calyx_core::CalyxError {
    calyx_core::CalyxError {
        code: "CALYX_ASTER_LATEST_ONLY_HISTORY_UNAVAILABLE",
        message: message.into(),
        remediation: "open the vault with full MVCC recovery before requesting historical row state",
    }
}

pub(super) fn hex_prefix(bytes: &[u8]) -> String {
    let mut value = String::new();
    for byte in bytes.iter().take(12) {
        value.push_str(&format!("{byte:02x}"));
    }
    if bytes.len() > 12 {
        value.push_str("...");
    }
    value
}
