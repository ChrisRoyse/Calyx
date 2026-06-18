use std::path::PathBuf;

use calyx_assay::{
    DeficitRoutingContext, PanelSufficiency, TrustTag, panel_sufficiency_with_context,
    per_sensor_attribution,
};
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CalyxError, Clock, FixedClock, LedgerRef, Panel, SlotId};
use calyx_lodestar::{LodestarError, RecallReport};
use calyx_oracle::{
    CalibrationMeasurement, CalibrationSource, DomainId, GoodhartDefenseMeasurement,
    GoodhartDefenseSource, HeldOutSplit, KernelRecallSource, MistakeClosureMeasurement,
    MistakeClosureSource, OracleConsistencySource, OracleError, OracleSelfConsistency,
    ShortCircuit, SufficiencyAssay, SuperIntelReport, SuperIntelligenceRequest,
    super_intelligence_with_ledger,
};

use super::vault_id;

#[derive(Clone)]
pub(super) struct SuperFixtures {
    oracle: OracleSource,
    assay: AssaySource,
    kernel: KernelSource,
    calibration: CalSource,
    goodhart: GoodSource,
    mistakes: MistakeSource,
}

impl SuperFixtures {
    pub(super) fn panel_fails() -> Self {
        let mut fixtures = Self::all_pass();
        fixtures.assay = AssaySource(panel_sufficiency(0.46, 1.0));
        fixtures
    }

    pub(super) fn all_pass() -> Self {
        Self {
            oracle: OracleSource(Ok(OracleSelfConsistency::with_provenance(
                0.0, 0.85, false, None,
            ))),
            assay: AssaySource(panel_sufficiency(1.05, 1.0)),
            kernel: KernelSource(Ok(recall_report(1.0, 2))),
            calibration: CalSource(Ok(CalibrationMeasurement {
                calibration_error: 0.20,
                held_out_count: 2,
                calibrated_slots: 1,
            })),
            goodhart: GoodSource(Ok(GoodhartDefenseMeasurement {
                pass_rate: 0.95,
                held_out_count: 2,
                report_passed: true,
                violation_count: 0,
            })),
            mistakes: MistakeSource(Ok(MistakeClosureMeasurement {
                recurring_mistakes: 0,
                replayed_mistakes: 4,
            })),
        }
    }

    pub(super) fn goodhart_source_fails() -> Self {
        let mut fixtures = Self::all_pass();
        fixtures.goodhart = GoodSource(Err(synthetic_error("CALYX_SYNTHETIC_GOODHART_DOWN")));
        fixtures
    }
}

pub(super) struct SuperCase {
    pub(super) report: SuperIntelReport,
    pub(super) ledger_b3: String,
}

pub(super) fn super_case(
    vault_dir: PathBuf,
    domain: &str,
    panel: &Panel,
    held_out: &HeldOutSplit,
    clock: &FixedClock,
    fixtures: SuperFixtures,
) -> SuperCase {
    let vault = durable_vault(&vault_dir, b"ph50-super");
    let request = SuperIntelligenceRequest {
        oracle: &fixtures.oracle,
        assay: &fixtures.assay,
        kernel: &fixtures.kernel,
        calibration: &fixtures.calibration,
        goodhart: &fixtures.goodhart,
        mistakes: &fixtures.mistakes,
        panel,
        domain: DomainId::from(domain),
        held_out,
        clock,
        short_circuit: ShortCircuit::MeasureAll,
    };
    let (report, ledger_ref) =
        super_intelligence_with_ledger(&vault, request).expect("super intel");
    vault.flush().expect("flush super vault");
    SuperCase {
        report,
        ledger_b3: ledger_b3(&vault, &ledger_ref),
    }
}

#[derive(Clone)]
struct OracleSource(Result<OracleSelfConsistency, OracleError>);

impl OracleConsistencySource for OracleSource {
    fn oracle_self_consistency(
        &self,
        _domain: DomainId,
        _clock: &dyn Clock,
    ) -> Result<OracleSelfConsistency, OracleError> {
        self.0.clone()
    }
}

#[derive(Clone)]
struct AssaySource(PanelSufficiency);

impl SufficiencyAssay for AssaySource {
    fn panel_sufficiency(
        &self,
        _panel: &Panel,
        _domain: &DomainId,
        _clock: &dyn Clock,
    ) -> Result<PanelSufficiency, OracleError> {
        Ok(self.0.clone())
    }
}

#[derive(Clone)]
struct KernelSource(Result<RecallReport, LodestarError>);

impl KernelRecallSource for KernelSource {
    fn kernel_recall_report(
        &self,
        held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<RecallReport, LodestarError> {
        let mut report = self.0.clone()?;
        report.held_out = held_out.held_out_ids.clone();
        Ok(report)
    }
}

#[derive(Clone)]
struct CalSource(Result<CalibrationMeasurement, OracleError>);

impl CalibrationSource for CalSource {
    fn calibration_measurement(
        &self,
        _domain: &DomainId,
        held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<CalibrationMeasurement, OracleError> {
        let mut measurement = self.0.clone()?;
        measurement.held_out_count = held_out.held_out_count();
        Ok(measurement)
    }
}

#[derive(Clone)]
struct GoodSource(Result<GoodhartDefenseMeasurement, OracleError>);

impl GoodhartDefenseSource for GoodSource {
    fn goodhart_defense_measurement(
        &self,
        _domain: &DomainId,
        held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<GoodhartDefenseMeasurement, OracleError> {
        let mut measurement = self.0.clone()?;
        measurement.held_out_count = held_out.held_out_count();
        Ok(measurement)
    }
}

#[derive(Clone)]
struct MistakeSource(Result<MistakeClosureMeasurement, OracleError>);

impl MistakeClosureSource for MistakeSource {
    fn mistake_closure_measurement(
        &self,
        _domain: &DomainId,
        _clock: &dyn Clock,
    ) -> Result<MistakeClosureMeasurement, OracleError> {
        self.0.clone()
    }
}

fn panel_sufficiency(panel_bits: f32, entropy_bits: f32) -> PanelSufficiency {
    panel_sufficiency_with_context(
        panel_bits,
        entropy_bits,
        &per_sensor_attribution(&[(SlotId::new(1), panel_bits.min(0.08))], 0.10),
        TrustTag::Trusted,
        DeficitRoutingContext {
            panel_id: "swe-bench-lite-form-only".to_string(),
            anchor: AnchorKind::Reward,
            computed_at_seq: 439,
            observation_scope: None,
        },
    )
}

fn durable_vault(path: &std::path::Path, salt: &[u8]) -> AsterVault {
    AsterVault::new_durable(path, vault_id(), salt.to_vec(), VaultOptions::default())
        .expect("open durable vault")
}

fn ledger_b3(vault: &AsterVault, ledger_ref: &LedgerRef) -> String {
    let row = vault
        .read_cf_at(
            vault.latest_seq(),
            ColumnFamily::Ledger,
            &ledger_key(ledger_ref.seq),
        )
        .expect("read ledger")
        .expect("ledger row");
    blake3::hash(&row).to_hex().to_string()
}

fn recall_report(ratio: f32, n_queries_tested: usize) -> RecallReport {
    RecallReport {
        kernel_only: ratio,
        full: 1.0,
        ratio,
        n_queries_tested,
        ..RecallReport::default()
    }
}

fn synthetic_error(code: &'static str) -> OracleError {
    CalyxError {
        code,
        message: "synthetic PH50 source unavailable".to_string(),
        remediation: "restore PH50 exit-gate source",
    }
    .into()
}
