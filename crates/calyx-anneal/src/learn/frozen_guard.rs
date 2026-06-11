use std::collections::HashMap;
use std::sync::Arc;

use calyx_core::{CalyxError, LensId, Result};
use calyx_registry::{FrozenLensSnapshot, Registry};
use serde::{Deserialize, Serialize};

pub const CALYX_REGISTRY_UNAVAILABLE: &str = "CALYX_REGISTRY_UNAVAILABLE";

pub trait FrozenLensSource: Send + Sync {
    fn frozen_lens_snapshots(&self) -> Result<Vec<FrozenLensSnapshot>>;
}

impl FrozenLensSource for Registry {
    fn frozen_lens_snapshots(&self) -> Result<Vec<FrozenLensSnapshot>> {
        Ok(Registry::frozen_lens_snapshots(self))
    }
}

pub trait FrozenLensCheck: Send + Sync {
    fn assert_no_violation(&self) -> Result<()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoFrozenLensGuard;

impl FrozenLensCheck for NoFrozenLensGuard {
    fn assert_no_violation(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct FrozenLensGuard<S = Registry> {
    registry: Arc<S>,
    known_hashes: HashMap<LensId, [u8; 32]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenCheckReport {
    pub ok: Vec<LensId>,
    pub violations: Vec<LensId>,
    pub new_lenses: Vec<LensId>,
    pub rows: Vec<FrozenLensReportRow>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenLensReportRow {
    pub lens_id: LensId,
    pub known_hash: Option<[u8; 32]>,
    pub observed_hash: [u8; 32],
    pub stable: bool,
    pub status: FrozenLensStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrozenLensStatus {
    Stable,
    New,
    Violation,
}

impl<S> FrozenLensGuard<S>
where
    S: FrozenLensSource,
{
    pub fn new(registry: Arc<S>) -> Self {
        Self {
            registry,
            known_hashes: HashMap::new(),
        }
    }

    pub fn initialize(&mut self) -> Result<()> {
        self.known_hashes.clear();
        for snapshot in self.registry.frozen_lens_snapshots()? {
            self.known_hashes
                .insert(snapshot.lens_id, weights_hash(snapshot));
        }
        Ok(())
    }

    pub fn check(&self) -> Result<FrozenCheckReport> {
        let mut rows = Vec::new();
        let mut ok = Vec::new();
        let mut violations = Vec::new();
        let mut new_lenses = Vec::new();
        for snapshot in self.registry.frozen_lens_snapshots()? {
            let observed_hash = weights_hash(snapshot);
            let known_hash = self.known_hashes.get(&snapshot.lens_id).copied();
            let status = match known_hash {
                Some(known) if known == observed_hash => {
                    ok.push(snapshot.lens_id);
                    FrozenLensStatus::Stable
                }
                Some(_) => {
                    violations.push(snapshot.lens_id);
                    FrozenLensStatus::Violation
                }
                None => {
                    new_lenses.push(snapshot.lens_id);
                    FrozenLensStatus::New
                }
            };
            rows.push(FrozenLensReportRow {
                lens_id: snapshot.lens_id,
                known_hash,
                observed_hash,
                stable: !matches!(status, FrozenLensStatus::Violation),
                status,
            });
        }
        rows.sort_by_key(|row| row.lens_id);
        ok.sort();
        violations.sort();
        new_lenses.sort();
        Ok(FrozenCheckReport {
            ok,
            violations,
            new_lenses,
            rows,
        })
    }

    pub fn assert_no_violation(&self) -> Result<()> {
        let report = self.check()?;
        if report.violations.is_empty() {
            return Ok(());
        }
        Err(CalyxError::lens_frozen_violation(format!(
            "frozen lens weights changed: {}",
            join_lens_ids(&report.violations)
        )))
    }

    pub fn report(&self) -> Result<Vec<(LensId, [u8; 32], bool)>> {
        Ok(self
            .check()?
            .rows
            .into_iter()
            .map(|row| (row.lens_id, row.observed_hash, row.stable))
            .collect())
    }

    pub fn known_hashes(&self) -> &HashMap<LensId, [u8; 32]> {
        &self.known_hashes
    }
}

impl<S> FrozenLensCheck for FrozenLensGuard<S>
where
    S: FrozenLensSource,
{
    fn assert_no_violation(&self) -> Result<()> {
        FrozenLensGuard::assert_no_violation(self)
    }
}

fn weights_hash(snapshot: FrozenLensSnapshot) -> [u8; 32] {
    snapshot.weights_sha256
}

fn join_lens_ids(ids: &[LensId]) -> String {
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}
