use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use calyx_core::{FixedClock, LensId, Result, Seq};

use super::{
    ComponentKind, DegradeRegistry, EndpointUrl, FaultDetector, FaultKind, HealthStorage,
    HttpProbe, LensProbeDetector, ProbeStatus,
};

#[test]
fn poisoned_lens_probe_state_emits_metrics_unavailable_fault() {
    let detector = LensProbeDetector::new(
        vec![(lens(7), EndpointUrl::new("http://tei/health"))],
        Arc::new(OkProbe),
        Arc::new(FixedClock::new(1_785_601_401)),
    );
    poison_probe_state(&detector);
    let registry = DegradeRegistry::open(
        Arc::new(FixedClock::new(1_785_601_401)),
        MemoryHealthStore::default(),
    )
    .unwrap();

    let events = detector.check(&registry);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].component, ComponentKind::lens_endpoint(lens(7)));
    assert_eq!(events[0].fault_kind, FaultKind::MetricsUnavailable);
}

fn poison_probe_state(detector: &LensProbeDetector) {
    let old_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let _guard = detector.state.lock().unwrap();
        panic!("poison lens probe state");
    }));
    std::panic::set_hook(old_hook);
}

struct OkProbe;

impl HttpProbe for OkProbe {
    fn probe(&self, _endpoint: &EndpointUrl) -> Result<ProbeStatus> {
        Ok(ProbeStatus { ok: true })
    }
}

#[derive(Clone, Default)]
struct MemoryHealthStore {
    rows: Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl HealthStorage for MemoryHealthStore {
    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<Seq> {
        let mut rows = self.rows.lock().unwrap();
        rows.insert(key, value);
        Ok(rows.len() as Seq)
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

fn lens(byte: u8) -> LensId {
    LensId::from_bytes([byte; 16])
}
