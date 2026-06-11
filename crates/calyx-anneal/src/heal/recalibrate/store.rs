use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{Result, SlotId};
use serde::{Deserialize, Serialize};

use super::types::{
    NewTau, WARD_TAU_TAG, WardTauReadback, WardTauStore, invalid_tau, validate_tau,
};
use crate::LogicalTime;

pub struct FileWardTauStore {
    path: PathBuf,
    rows: BTreeMap<SlotId, WardTauReadback>,
}

impl FileWardTauStore {
    pub fn open(vault: impl AsRef<Path>) -> Result<Self> {
        let path = ward_tau_path(vault.as_ref());
        if !path.exists() {
            return Ok(Self {
                path,
                rows: BTreeMap::new(),
            });
        }
        let bytes = fs::read(&path)
            .map_err(|error| invalid_tau(format!("read {}: {error}", path.display())))?;
        let file = serde_json::from_slice::<WardTauFile>(&bytes)
            .map_err(|error| invalid_tau(format!("decode {}: {error}", path.display())))?;
        if file.tag != WARD_TAU_TAG {
            return Err(invalid_tau("ward tau file tag mismatch"));
        }
        let mut rows = BTreeMap::new();
        for row in file.slots {
            validate_tau(row.tau)?;
            rows.insert(row.slot_id, row);
        }
        Ok(Self { path, rows })
    }

    pub fn upsert_current(
        &mut self,
        slot_id: SlotId,
        tau: f32,
        updated_at: LogicalTime,
    ) -> Result<()> {
        validate_tau(tau)?;
        self.rows.insert(
            slot_id,
            WardTauReadback {
                slot_id,
                tau,
                far: 0.0,
                frr: 0.0,
                updated_at,
            },
        );
        self.persist()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn persist(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| invalid_tau(format!("create {}: {error}", parent.display())))?;
        }
        let file = WardTauFile {
            tag: WARD_TAU_TAG.to_string(),
            slots: self.rows.values().cloned().collect(),
        };
        let bytes = serde_json::to_vec_pretty(&file)
            .map_err(|error| invalid_tau(format!("encode ward tau file: {error}")))?;
        fs::write(&self.path, bytes)
            .map_err(|error| invalid_tau(format!("write {}: {error}", self.path.display())))
    }
}

impl WardTauStore for FileWardTauStore {
    fn current_tau(&self, slot_id: SlotId) -> Result<Option<f32>> {
        Ok(self.rows.get(&slot_id).map(|row| row.tau))
    }

    fn set_live_tau(
        &mut self,
        slot_id: SlotId,
        tau: &NewTau,
        updated_at: LogicalTime,
    ) -> Result<()> {
        if tau.slot_id != slot_id {
            return Err(invalid_tau("new tau slot_id does not match target slot"));
        }
        self.rows.insert(
            slot_id,
            WardTauReadback {
                slot_id,
                tau: tau.tau,
                far: tau.far,
                frr: tau.frr,
                updated_at,
            },
        );
        self.persist()
    }

    fn readback(&self) -> Result<Vec<WardTauReadback>> {
        Ok(self.rows.values().cloned().collect())
    }
}

#[derive(Serialize, Deserialize)]
struct WardTauFile {
    tag: String,
    slots: Vec<WardTauReadback>,
}

pub fn ward_tau_path(vault: &Path) -> PathBuf {
    vault.join(".anneal").join("ward_tau.json")
}
