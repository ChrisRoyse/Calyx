use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use calyx_assay::{AssayCacheKey, AssayStore, AssaySubject, EstimatorKind, MiEstimate, TrustTag};
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{OccurrenceContext, RetentionPolicy, append_occurrence, read_series};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AnchorKind, AnchorValue, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef,
    Modality, Panel, SlotId, VaultId, VaultStore,
};
use calyx_oracle::{
    CalibrationMeasurement, DomainId, GoodhartDefenseMeasurement, HeldOutSplit,
    MistakeClosureMeasurement, ORACLE_DOMAIN_METADATA_KEY, ORACLE_FALLBACK_DOMAIN_METADATA_KEY,
    ShortCircuit, SuperIntelligenceRequest, VaultSufficiencyAssay, super_intelligence_with_ledger,
};
use serde::Deserialize;
use serde_json::json;

use crate::cf_read::hex_bytes;

mod sources;
use sources::{
    CalibrationSourceFixture, GoodhartSourceFixture, KernelFixture, KernelSource,
    MistakeSourceFixture, OracleFixture, validate_bits, validate_fraction,
};

const USAGE: &str = "usage: calyx readback super_intelligence --vault <dir> --domain <domain> --fixture <json> --vault-id <id> --salt <s>";

pub(crate) fn readback_super_intelligence(args: &[String]) -> crate::error::CliResult {
    let args = ReadbackArgs::parse(args)?;
    let fixture = SuperIntelFixture::read(&args.fixture, &args.domain)?;
    let vault_id = VaultId::from_str(&args.vault_id)
        .map_err(|error| format!("invalid --vault-id: {error}"))?;
    let vault = AsterVault::new_durable(
        &args.vault,
        vault_id,
        args.salt.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;

    let oracle_rows = fixture.persist_oracle_rows(&vault, vault_id, &args.domain)?;
    let assay_rows = fixture
        .sufficiency
        .persist_assay_rows(&vault, &fixture, &args.domain)?;
    let clock = FixedClock::new(fixture.clock_ts);
    let assay = VaultSufficiencyAssay::new(&vault);
    let kernel = KernelSource(fixture.kernel.clone());
    let calibration = CalibrationSourceFixture(fixture.calibration.clone());
    let goodhart = GoodhartSourceFixture(fixture.goodhart.clone());
    let mistakes = MistakeSourceFixture(fixture.mistake.clone());
    let domain = DomainId::from(args.domain.clone());

    let request = SuperIntelligenceRequest {
        oracle: &vault,
        assay: &assay,
        kernel: &kernel,
        calibration: &calibration,
        goodhart: &goodhart,
        mistakes: &mistakes,
        panel: &fixture.panel,
        domain,
        held_out: &fixture.held_out,
        clock: &clock,
        short_circuit: ShortCircuit::MeasureAll,
    };

    match super_intelligence_with_ledger(&vault, request) {
        Ok((report, ledger_ref)) => {
            vault
                .flush()
                .map_err(|error| format!("flush super_intelligence readback: {error}"))?;
            let ledger_row = read_ledger_row(&vault, &ledger_ref)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "source_of_truth": {
                        "vault": args.vault,
                        "oracle_base_rows_written": oracle_rows.base_rows,
                        "oracle_recurrence_rows_written": oracle_rows.recurrence_rows,
                        "assay_rows_written": assay_rows,
                        "ledger_ref": ledger_ref,
                        "ledger_key_hex": hex_bytes(&ledger_key(ledger_ref.seq)),
                        "ledger_value_b3": hex_bytes(blake3::hash(&ledger_row).as_bytes()),
                        "ledger_value_len": ledger_row.len(),
                    },
                    "report": report,
                }))
                .map_err(|error| error.to_string())?
            );
            Ok(())
        }
        Err(error) => {
            let _ = vault.flush();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "domain": args.domain,
                    "oracle_base_rows_written": oracle_rows.base_rows,
                    "oracle_recurrence_rows_written": oracle_rows.recurrence_rows,
                    "assay_rows_written": assay_rows,
                    "error_code": error.code(),
                    "error": error.to_string(),
                    "remediation": error.remediation(),
                }))
                .map_err(|error| error.to_string())?
            );
            Err(error.to_string().into())
        }
    }
}

#[derive(Debug)]
struct ReadbackArgs {
    vault: PathBuf,
    domain: String,
    fixture: PathBuf,
    vault_id: String,
    salt: String,
}

impl ReadbackArgs {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut vault = None;
        let mut domain = None;
        let mut fixture = None;
        let mut vault_id = None;
        let mut salt = None;
        let mut index = 0;
        while index < args.len() {
            let flag = args[index].as_str();
            let value = args.get(index + 1).ok_or_else(|| USAGE.to_string())?;
            match flag {
                "--vault" => vault = Some(PathBuf::from(value)),
                "--domain" => domain = Some(value.clone()),
                "--fixture" => fixture = Some(PathBuf::from(value)),
                "--vault-id" => vault_id = Some(value.clone()),
                "--salt" => salt = Some(value.clone()),
                _ => return Err(USAGE.to_string()),
            }
            index += 2;
        }
        Ok(Self {
            vault: vault.ok_or_else(|| USAGE.to_string())?,
            domain: domain.ok_or_else(|| USAGE.to_string())?,
            fixture: fixture.ok_or_else(|| USAGE.to_string())?,
            vault_id: vault_id.ok_or_else(|| USAGE.to_string())?,
            salt: salt.ok_or_else(|| USAGE.to_string())?,
        })
    }
}

#[derive(Debug, Deserialize)]
struct SuperIntelFixture {
    #[serde(default)]
    domain: Option<String>,
    panel: Panel,
    held_out: HeldOutSplit,
    sufficiency: SufficiencyFixture,
    #[serde(default)]
    oracle: OracleFixture,
    kernel: KernelFixture,
    calibration: CalibrationMeasurement,
    goodhart: GoodhartDefenseMeasurement,
    mistake: MistakeClosureMeasurement,
    clock_ts: u64,
}

impl SuperIntelFixture {
    fn read(path: &Path, domain: &str) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|error| format!("read fixture: {error}"))?;
        let fixture: Self =
            serde_json::from_slice(&bytes).map_err(|error| format!("parse fixture: {error}"))?;
        if fixture
            .domain
            .as_deref()
            .is_some_and(|value| value != domain)
        {
            return Err("fixture domain does not match --domain".to_string());
        }
        fixture.validate()?;
        Ok(fixture)
    }

    fn validate(&self) -> Result<(), String> {
        self.sufficiency.validate()?;
        self.kernel.validate()?;
        if !self.calibration.calibration_error.is_finite()
            || self.calibration.calibration_error < 0.0
        {
            return Err("calibration_error must be finite and non-negative".to_string());
        }
        validate_fraction(self.goodhart.pass_rate, "goodhart.pass_rate")?;
        Ok(())
    }

    fn persist_oracle_rows(
        &self,
        vault: &AsterVault,
        vault_id: VaultId,
        domain: &str,
    ) -> Result<OracleRowsWritten, String> {
        let raw = format!("super-intelligence-oracle-row:{domain}");
        let cx_id = vault.cx_id_for_input(raw.as_bytes(), self.panel.version.max(1));
        vault
            .put(oracle_constellation(
                vault,
                vault_id,
                cx_id,
                raw.as_bytes(),
                &self.panel,
                domain,
                self.clock_ts,
            ))
            .map_err(|error| error.to_string())?;

        let context = oracle_context(
            &self.oracle.oracle_verdict,
            &self.oracle.ground_truth_anchor,
        )?;
        for offset in 0..self.oracle.occurrence_count {
            let at = epoch(self.clock_ts.saturating_add(offset as u64))?;
            append_occurrence(
                vault,
                cx_id,
                at,
                OccurrenceContext::new(context.clone()).map_err(|error| error.to_string())?,
                at,
                RetentionPolicy::default(),
            )
            .map_err(|error| error.to_string())?;
        }
        let series = read_series(vault, cx_id).map_err(|error| error.to_string())?;
        Ok(OracleRowsWritten {
            base_rows: 1,
            recurrence_rows: series.occurrences.len(),
        })
    }
}

#[derive(Debug, Deserialize)]
struct SufficiencyFixture {
    #[serde(rename = "I_panel_oracle", alias = "panel_bits")]
    panel_bits: f32,
    outcome_entropy_bits: f32,
    slot_bits: Vec<FixtureSlotBits>,
    #[serde(default = "sources::default_samples")]
    n_samples: usize,
    #[serde(default = "sources::default_trust")]
    trust: TrustTag,
}

impl SufficiencyFixture {
    fn validate(&self) -> Result<(), String> {
        validate_bits(self.panel_bits, "I_panel_oracle")?;
        validate_bits(self.outcome_entropy_bits, "outcome_entropy_bits")?;
        for slot in &self.slot_bits {
            validate_bits(slot.bits, "slot_bits")?;
        }
        if self.n_samples == 0 {
            return Err("super_intelligence fixture n_samples must be positive".to_string());
        }
        Ok(())
    }

    fn persist_assay_rows(
        &self,
        vault: &AsterVault,
        fixture: &SuperIntelFixture,
        domain: &str,
    ) -> Result<usize, String> {
        let key = AssayCacheKey::scoped(
            fixture.panel.version,
            domain,
            vault.vault_id(),
            AnchorKind::Reward,
        );
        let mut store = AssayStore::default();
        store.put(
            key.clone(),
            AssaySubject::Panel,
            self.estimate(self.panel_bits, EstimatorKind::PanelSufficiency),
            "super_intelligence panel bits fixture",
            fixture.clock_ts,
        );
        store.put(
            key.clone(),
            AssaySubject::OutcomeEntropy,
            self.estimate(self.outcome_entropy_bits, EstimatorKind::OutcomeEntropy),
            "super_intelligence outcome entropy fixture",
            fixture.clock_ts,
        );
        for slot in &self.slot_bits {
            store.put(
                key.clone(),
                AssaySubject::Lens { slot: slot.slot },
                self.estimate(slot.bits, EstimatorKind::Ksg),
                "super_intelligence lens bits fixture",
                fixture.clock_ts,
            );
        }
        store
            .persist_to_vault(vault)
            .map_err(|error| error.to_string())
    }

    fn estimate(&self, bits: f32, estimator: EstimatorKind) -> MiEstimate {
        MiEstimate::point(bits, self.n_samples, estimator, self.trust)
    }
}

#[derive(Debug, Deserialize)]
struct FixtureSlotBits {
    slot: SlotId,
    bits: f32,
}

#[derive(Debug)]
struct OracleRowsWritten {
    base_rows: usize,
    recurrence_rows: usize,
}

fn oracle_constellation(
    vault: &AsterVault,
    vault_id: VaultId,
    cx_id: CxId,
    raw: &[u8],
    panel: &Panel,
    domain: &str,
    clock_ts: u64,
) -> Constellation {
    let metadata = BTreeMap::from([
        (ORACLE_DOMAIN_METADATA_KEY.to_string(), domain.to_string()),
        (
            ORACLE_FALLBACK_DOMAIN_METADATA_KEY.to_string(),
            domain.to_string(),
        ),
    ]);
    Constellation {
        cx_id,
        vault_id,
        panel_version: panel.version.max(1),
        created_at: clock_ts,
        input_ref: InputRef {
            hash: *blake3::hash(raw).as_bytes(),
            pointer: Some("synthetic://super-intelligence-oracle".to_string()),
            redacted: true,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: vault.latest_seq(),
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: false,
            redacted_input: true,
            ..CxFlags::default()
        },
    }
}

fn oracle_context(verdict: &AnchorValue, truth: &AnchorValue) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({
        "oracle_verdict": { "value": verdict },
        "ground_truth_anchor": { "value": truth },
    }))
    .map_err(|error| error.to_string())
}

fn read_ledger_row(vault: &AsterVault, ledger_ref: &LedgerRef) -> Result<Vec<u8>, String> {
    vault
        .read_cf_at(
            vault.latest_seq(),
            ColumnFamily::Ledger,
            &ledger_key(ledger_ref.seq),
        )
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("ledger row {} not found", ledger_ref.seq))
}

fn epoch(value: u64) -> Result<EpochSecs, String> {
    i64::try_from(value)
        .map(EpochSecs)
        .map_err(|_| "fixture clock_ts is too large for EpochSecs".to_string())
}
