use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    ActionMetricSnapshot, ArtifactKey, ArtifactPtr, ArtifactReplayMeasurer, BudgetProbe,
    BudgetProbeSample, ReplayQuery, RollbackStorage, TripwireMetric,
};
use calyx_core::{Result, Seq};

#[derive(Clone, Default)]
pub(crate) struct MemoryRollbackStorage {
    rows: Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl RollbackStorage for MemoryRollbackStorage {
    fn put_many(&self, rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<Seq> {
        let mut inner = self.rows.lock().unwrap();
        for (key, value) in rows {
            inner.insert(key, value);
        }
        Ok(inner.len() as Seq)
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.rows.lock().unwrap().get(key).cloned())
    }

    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }
}

#[derive(Clone)]
pub(crate) struct ScriptedProbe;

impl BudgetProbe for ScriptedProbe {
    fn sample(&self) -> BudgetProbeSample {
        BudgetProbeSample {
            cpu_used_fraction: 0.0,
            vram_used_bytes: 0,
            nvml_available: true,
            warning_code: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct ScriptedEqualArtifactMeasurer;

impl ArtifactReplayMeasurer for ScriptedEqualArtifactMeasurer {
    fn measure(
        &self,
        _key: &ArtifactKey,
        _artifact: &ArtifactPtr,
        _query: &ReplayQuery,
    ) -> Result<ActionMetricSnapshot> {
        Ok(ActionMetricSnapshot::from_values([
            (TripwireMetric::RecallAtK, 0.95),
            (TripwireMetric::GuardFAR, 0.001),
            (TripwireMetric::GuardFRR, 0.001),
            (TripwireMetric::SearchP99, 50.0),
            (TripwireMetric::IngestP95, 80.0),
        ]))
    }
}
