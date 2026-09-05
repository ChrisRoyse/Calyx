use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Clock, Constellation, CxFlags, CxId, InputRef, LedgerRef,
    Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_lodestar::{KernelGraphParams, KernelParams, RecallQuery, RecallTestParams};
use calyx_poly::Domain;
use calyx_poly::admission::AdmissionInputs;
use calyx_poly::calibration_refit::{CalibrationRefitObservation, CalibrationRefitRequest};
use calyx_poly::daily_ops_scheduler::{
    DailyOpsSchedulerConfig, DailyOpsSchedulerRequest, ERR_DAILY_OPS_SCHEDULER_INVALID_CONFIG,
    read_daily_ops_scheduler_state, run_daily_ops_scheduler_tick,
};
use calyx_poly::domain_graph_build_job::{DomainGraphBuildRequest, DomainGraphEdgeInput};
use calyx_poly::entity_graph_edges::EDGE_SHARED_ENTITY;
use calyx_poly::knn_graph_edges::EDGE_KNN_RESOLVED;
use calyx_poly::panel_diagnostics::PanelMatrix;
use calyx_poly::policy::LocalOnlyPolicy;
use calyx_poly::structural_edges::EDGE_YES_NO_COMPLEMENT;
use calyx_poly::temporal_graph_edges::EDGE_TEMPORAL_LEAD_LAG;
use calyx_poly::ward_calibration::{
    WardCalibrationRequest, WardCalibrationResidual, WardResidualClass,
};
use calyx_ward::{MIN_BAD_SCORES, SlotKind};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;

use support::{
    known_healthy_market_integrity, known_healthy_oracle_risk, known_healthy_wash_trade,
};

pub const VAULT_SALT: &[u8] = b"poly-issue012-daily-ops-scheduler";
pub const TEST_TS: u64 = 1_785_501_212;
pub const AS_OF_MILLIS: u64 = 1_785_501_212_000;

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const COLLECTION: &str = "poly_issue012_daily_ops_graph";
const GUARD_ID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c191";
const WARD_SLOT: SlotId = SlotId::new(9);

pub struct DailyOpsFixturePaths {
    pub root: PathBuf,
    pub graph_dir: PathBuf,
    pub ward_dir: PathBuf,
    pub calibration_dir: PathBuf,
}

impl DailyOpsFixturePaths {
    pub fn new(root: PathBuf) -> Self {
        Self {
            graph_dir: root.join("domain-graph"),
            ward_dir: root.join("ward"),
            calibration_dir: root.join("calibration"),
            root,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn scheduler_request<'a, C: Clock>(
    paths: &'a DailyOpsFixturePaths,
    vault: &'a AsterVault<C>,
    source_cx_ids: &'a [CxId],
    config: DailyOpsSchedulerConfig,
    now_ts: u64,
    clock: &'a dyn Clock,
) -> DailyOpsSchedulerRequest<'a, C> {
    DailyOpsSchedulerRequest {
        output_root: &paths.root,
        config,
        now_ts,
        policy: LocalOnlyPolicy::default(),
        vault,
        clock,
        domain_graph: domain_graph_request(&paths.graph_dir, source_cx_ids),
        ward: ward_request("issue012-daily"),
        ward_output_dir: &paths.ward_dir,
        calibration_refit: calibration_request(&paths.calibration_dir),
    }
}

pub fn scheduler_config() -> DailyOpsSchedulerConfig {
    DailyOpsSchedulerConfig {
        job_id: "issue012-daily-ops".to_string(),
        cadence_secs: 86_400,
    }
}

pub fn edge_invalid_config<C: Clock>(
    paths: &DailyOpsFixturePaths,
    vault: &AsterVault<C>,
    source_cx_ids: &[CxId],
    config: DailyOpsSchedulerConfig,
    clock: &dyn Clock,
) -> Value {
    let state_path = paths.root.join("daily-ops-scheduler-state.json");
    let before = read_daily_ops_scheduler_state(&state_path).expect("read state before edge");
    let err = run_daily_ops_scheduler_tick(scheduler_request(
        paths,
        vault,
        source_cx_ids,
        config,
        TEST_TS,
        clock,
    ))
    .expect_err("invalid scheduler config must fail closed");
    assert_eq!(err.code(), ERR_DAILY_OPS_SCHEDULER_INVALID_CONFIG);
    let after = read_daily_ops_scheduler_state(&state_path).expect("read state after edge");
    assert_eq!(before, after);
    json!({
        "code": err.code(),
        "message": err.message(),
        "before": before,
        "after": after,
        "state_unchanged": true
    })
}

pub fn store_loom_constellations(vault: &AsterVault) -> Vec<CxId> {
    [70u8, 71u8]
        .iter()
        .map(|id| {
            vault
                .put(constellation(*id, vault.vault_id()))
                .expect("put constellation")
        })
        .collect()
}

pub fn open_vault(dir: &Path) -> AsterVault {
    AsterVault::open(
        dir,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open daily ops scheduler vault")
}

pub fn assert_c_drive(path: &Path) {
    #[cfg(not(windows))]
    let _ = path;
    #[cfg(windows)]
    assert!(
        path.to_string_lossy().starts_with("C:"),
        "{} must stay on C:",
        path.display()
    );
}

fn domain_graph_request<'a>(
    graph_dir: &'a Path,
    source_cx_ids: &'a [CxId],
) -> DomainGraphBuildRequest<'a> {
    DomainGraphBuildRequest {
        domain: Domain::Crypto,
        collection: COLLECTION,
        panel_version: 12,
        source_cx_ids,
        supplied_edges: Box::leak(Box::new(supplied_edges())),
        pair_gain_matrix: Box::leak(Box::new(pair_gain_matrix())),
        recall_corpus: Box::leak(Box::new(recall_corpus())),
        kernel_anchors: Box::leak(Box::new(vec![cx(1)])),
        kernel_params: Box::leak(Box::new(kernel_params())),
        recall_params: Box::leak(Box::new(recall_params())),
        output_dir: graph_dir,
        loom_cache_capacity: 64,
    }
}

fn ward_request(name: &str) -> WardCalibrationRequest {
    WardCalibrationRequest {
        calibration_version: format!("{name}-v1"),
        domain: "crypto".to_string(),
        horizon_bucket: "1h_24h".to_string(),
        panel_version: 12,
        guard_id: GUARD_ID.to_string(),
        slot: WARD_SLOT,
        slot_kind: SlotKind::Content,
        target_far: 0.03,
        alpha: 0.05,
        min_anchor_count: MIN_BAD_SCORES,
        max_age_seconds: 3_600,
        now_ts: TEST_TS as i64 + 60,
        candidate_score: 1.0,
        residuals: anchored_residuals(),
        admission_params: Default::default(),
        admission_inputs: good_inputs(),
    }
}

fn calibration_request(out_dir: &Path) -> CalibrationRefitRequest<'_> {
    CalibrationRefitRequest {
        out_dir,
        domain: "crypto",
        horizon_bucket: "1h_24h",
        previous_version: Some("crypto:1h_24h:previous"),
        as_of_millis: AS_OF_MILLIS,
        observations: calibration_observations(),
    }
}

fn supplied_edges() -> Vec<DomainGraphEdgeInput> {
    vec![
        edge(
            1,
            2,
            EDGE_SHARED_ENTITY,
            "entity:a-b",
            "entity_graph_edges",
            true,
        ),
        edge(
            2,
            3,
            EDGE_TEMPORAL_LEAD_LAG,
            "temporal:b-c",
            "temporal_graph_edges",
            true,
        ),
        edge(3, 1, EDGE_KNN_RESOLVED, "knn:c-a", "knn_graph_edges", true),
        edge(
            4,
            5,
            EDGE_YES_NO_COMPLEMENT,
            "structural:d-e",
            "structural_edges",
            false,
        ),
    ]
}

fn edge(
    src: u8,
    dst: u8,
    edge_type: &str,
    relation_key: &str,
    source: &str,
    include_in_kernel: bool,
) -> DomainGraphEdgeInput {
    DomainGraphEdgeInput {
        src: cx(src),
        dst: cx(dst),
        edge_type: edge_type.to_string(),
        relation_key: relation_key.to_string(),
        source: source.to_string(),
        weight: 0.9,
        include_in_kernel,
    }
}

fn pair_gain_matrix() -> PanelMatrix {
    PanelMatrix::new(
        vec!["slot_a".to_string(), "slot_b".to_string()],
        vec![vec![0.1, 0.2, 0.3, 0.4], vec![0.4, 0.3, 0.2, 0.1]],
        vec![
            bool_anchor(false, 1),
            bool_anchor(true, 2),
            bool_anchor(false, 3),
            bool_anchor(true, 4),
        ],
    )
    .expect("pair-gain matrix")
}

fn recall_corpus() -> Vec<RecallQuery> {
    vec![
        RecallQuery {
            cx_id: cx(1),
            vector: vec![1.0, 0.0, 0.0],
        },
        RecallQuery {
            cx_id: cx(2),
            vector: vec![0.0, 1.0, 0.0],
        },
        RecallQuery {
            cx_id: cx(3),
            vector: vec![0.0, 0.0, 1.0],
        },
    ]
}

fn kernel_params() -> KernelParams {
    KernelParams {
        kernel_graph: KernelGraphParams {
            target_fraction: 1.0,
            max_groundedness_distance: 64,
            ..KernelGraphParams::default()
        },
        ..KernelParams::default()
    }
}

fn recall_params() -> RecallTestParams {
    RecallTestParams {
        held_out_fraction: 1.0,
        top_k: 3,
        rng_seed: 12,
        min_recall_ratio: 0.95,
    }
}

fn anchored_residuals() -> Vec<WardCalibrationResidual> {
    let mut residuals = (0..MIN_BAD_SCORES)
        .map(|idx| WardCalibrationResidual {
            slot: WARD_SLOT,
            class: WardResidualClass::KnownBad,
            score: 0.10 + (idx as f32 * 0.001),
        })
        .collect::<Vec<_>>();
    residuals.push(WardCalibrationResidual {
        slot: WARD_SLOT,
        class: WardResidualClass::KnownGood,
        score: 0.95,
    });
    residuals
}

fn calibration_observations() -> Vec<CalibrationRefitObservation> {
    let mut rows = Vec::new();
    for i in 0..15 {
        rows.push(cal_obs(0.60, i % 5 != 0, i));
    }
    for i in 0..15 {
        rows.push(cal_obs(0.40, i % 5 == 0, 15 + i));
    }
    rows
}

fn cal_obs(p_raw: f64, outcome_yes: bool, offset: u64) -> CalibrationRefitObservation {
    CalibrationRefitObservation {
        p_raw,
        outcome_yes,
        resolved_at_millis: AS_OF_MILLIS - 30_000 + offset,
    }
}

fn good_inputs() -> AdmissionInputs {
    AdmissionInputs {
        p_win: 0.94,
        confidence: 0.74,
        sufficiency_ok: true,
        evidence_count: 2,
        source_derived_evidence_count: 2,
        stale_evidence_count: 0,
        circular_evidence_count: 0,
        super_intel_pass: true,
        guard_calibrated: false,
        grounding_anchor_count: 0,
        guard_pass: false,
        liquidity_ok: true,
        market_integrity: known_healthy_market_integrity(),
        oracle_risk: known_healthy_oracle_risk(),
        wash_trade: known_healthy_wash_trade(),
        kill_switch_active: false,
        daily_error_score: 0.0,
    }
}

fn constellation(id: u8, vault_id: VaultId) -> Constellation {
    let cx_id = cx(id);
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(1),
        SlotVector::Dense {
            dim: 2,
            data: vec![1.0, id as f32 / 100.0],
        },
    );
    slots.insert(
        SlotId::new(2),
        SlotVector::Dense {
            dim: 2,
            data: vec![0.8, id as f32 / 120.0],
        },
    );
    Constellation {
        cx_id,
        vault_id,
        panel_version: 12,
        created_at: TEST_TS,
        input_ref: InputRef {
            hash: [id; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Structured,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

fn bool_anchor(value: bool, observed_at: u64) -> Anchor {
    Anchor {
        kind: AnchorKind::TestPass,
        value: AnchorValue::Bool(value),
        source: "uma:issue012-daily".to_string(),
        observed_at,
        confidence: 1.0,
    }
}

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("vault id")
}
