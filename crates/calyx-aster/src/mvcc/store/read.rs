use super::*;

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
                .get(&(cf, key.to_vec()))
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
            .get(&(cf, key.to_vec()))
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
        for read in reads {
            self.ensure_unbarriered(read.cf, &read.key)?;
        }
        reads
            .iter()
            .map(|read| self.read_at(snapshot, read.cf, &read.key, clock))
            .collect()
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
        let start = after_key.unwrap_or(&range.start).to_vec();
        let lower = if after_key.is_some() {
            Bound::Excluded((cf, start))
        } else {
            Bound::Included((cf, start))
        };
        let table = self.rows.read().expect("mvcc row table poisoned");
        let mut rows = Vec::with_capacity(limit);
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

    fn ensure_unbarriered(&self, cf: ColumnFamily, key: &[u8]) -> Result<()> {
        let barriers = self
            .read_barriers
            .read()
            .expect("mvcc read barriers poisoned");
        if let Some(error) = first_blocking(&barriers, cf, key) {
            return Err(error);
        }
        Ok(())
    }

    fn router_latest_value(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        if !self.router_latest_readback.load(Ordering::Acquire) {
            return Ok(None);
        }
        self.ensure_router_latest_snapshot(snapshot)?;
        let router = self.router.read().expect("mvcc router poisoned");
        let Some(router) = router.as_ref() else {
            return Ok(None);
        };
        Ok(router
            .get(cf, key)?
            .filter(|value| !is_tombstone_value(value)))
    }

    fn router_latest_rows(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        range: Option<&KeyRange>,
    ) -> Result<BTreeMap<Vec<u8>, Vec<u8>>> {
        if !self.router_latest_readback.load(Ordering::Acquire) {
            return Ok(BTreeMap::new());
        }
        self.ensure_router_latest_snapshot(snapshot)?;
        let router = self.router.read().expect("mvcc router poisoned");
        let Some(router) = router.as_ref() else {
            return Ok(BTreeMap::new());
        };
        let rows = match range {
            Some(range) => match range.end.as_deref() {
                Some(end) => router.range(cf, &range.start, end)?,
                None => router
                    .iter_cf(cf)?
                    .into_iter()
                    .filter(|row| row.key.as_slice() >= range.start.as_slice())
                    .collect(),
            },
            None => router.iter_cf(cf)?,
        };
        Ok(rows
            .into_iter()
            .filter_map(|row| (!is_tombstone_value(&row.value)).then_some((row.key, row.value)))
            .collect())
    }

    fn router_latest_keys(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        range: &KeyRange,
    ) -> Result<BTreeMap<Vec<u8>, ()>> {
        if !self.router_latest_readback.load(Ordering::Acquire) {
            return Ok(BTreeMap::new());
        }
        self.ensure_router_latest_snapshot(snapshot)?;
        let router = self.router.read().expect("mvcc router poisoned");
        let Some(router) = router.as_ref() else {
            return Ok(BTreeMap::new());
        };
        Ok(router
            .range_keys_until(cf, &range.start, range.end.as_deref())?
            .into_iter()
            .map(|key| (key, ()))
            .collect())
    }

    fn overlay_table_rows(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        range: Option<&KeyRange>,
        rows: &mut BTreeMap<Vec<u8>, Vec<u8>>,
    ) {
        let table = self.rows.read().expect("mvcc row table poisoned");
        for ((row_cf, key), versions) in table.iter() {
            if *row_cf != cf || range.is_some_and(|range| !range.contains(key)) {
                continue;
            }
            match visible_value_state(versions, snapshot.seq()) {
                Some(VisibleValue::Live(value)) => {
                    rows.insert(key.clone(), value);
                }
                Some(VisibleValue::Tombstone) => {
                    rows.remove(key);
                }
                None => {}
            }
        }
    }

    fn overlay_table_keys(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        range: &KeyRange,
        keys: &mut BTreeMap<Vec<u8>, ()>,
    ) {
        let table = self.rows.read().expect("mvcc row table poisoned");
        for ((row_cf, key), versions) in table.iter() {
            if *row_cf != cf || !range.contains(key) {
                continue;
            }
            match visible_value_state(versions, snapshot.seq()) {
                Some(VisibleValue::Live(_)) => {
                    keys.insert(key.clone(), ());
                }
                Some(VisibleValue::Tombstone) => {
                    keys.remove(key);
                }
                None => {}
            }
        }
    }

    fn ensure_router_latest_snapshot(&self, snapshot: Snapshot) -> Result<()> {
        let latest = self.current_seq();
        if snapshot.seq() == latest {
            return Ok(());
        }
        Err(latest_only_error(format!(
            "historical snapshot {} requested from latest-only recovered vault at seq {}",
            snapshot.seq(),
            latest
        )))
    }

    fn ensure_snapshot_live(&self, snapshot: Snapshot, clock: &dyn Clock) -> Result<()> {
        let now = clock.now();
        let lease = snapshot.lease();
        if lease.is_expired_at(now) {
            self.leases.abort_if_expired(lease, now);
        }
        lease.ensure_live_at(now)
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

fn hex_prefix(bytes: &[u8]) -> String {
    let mut value = String::new();
    for byte in bytes.iter().take(12) {
        value.push_str(&format!("{byte:02x}"));
    }
    if bytes.len() > 12 {
        value.push_str("...");
    }
    value
}
