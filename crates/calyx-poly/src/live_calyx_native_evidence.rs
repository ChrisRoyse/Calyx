//! Aster-backed measurement evidence for live CalyxNative forecast admission.

use calyx_anneal::{GoodhartReport, HeldOutSet, RegressionReport};
use calyx_assay::MIN_ASSAY_SAMPLES;
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, LedgerRef};
use calyx_ledger::{ActorId, EntryKind, SubjectId, decode as decode_ledger};
use calyx_oracle::GOODHART_THRESHOLD;
use serde::{Deserialize, Serialize};

use crate::calibration_refit::{
    CALIBRATION_REFIT_ARTIFACT_KIND, CALIBRATION_REFIT_SCHEMA_VERSION, CalibrationRefitReport,
};
use crate::calyx_native::CalyxNativeEvidenceRef;
use crate::forecast_calibration::MIN_CALIBRATION_SAMPLES;
use crate::kernel_recall::POLY_KERNEL_RECALL_MIN_RATIO;
use crate::kernel_recall_admission::{
    COMPUTED_KERNEL_RECALL_ARTIFACT_KIND, COMPUTED_KERNEL_RECALL_SCHEMA_VERSION,
    ComputedKernelRecall,
};
use crate::panel_sufficiency::{
    POLY_PANEL_SUFFICIENCY_ARTIFACT_KIND, POLY_PANEL_SUFFICIENCY_SCHEMA_VERSION,
    PolyPanelSufficiencyReport,
};
use crate::{PolyError, Result};

mod codec;
mod validation;
use codec::{decode_payload, encode_payload};
use validation::validate_evidence;

pub const LIVE_CALYX_NATIVE_EVIDENCE_SCHEMA_VERSION: &str = "poly.live_calyx_native_evidence.v1";
pub const LIVE_CALYX_NATIVE_EVIDENCE_EVENT: &str = "poly.live_native_evidence.recorded";
pub const LIVE_CALYX_NATIVE_EVIDENCE_MAX_AGE_MILLIS: u64 = 86_400_000;
pub const LIVE_CALYX_NATIVE_MIN_PANEL_LENSES: usize = 10;

pub const ERR_LIVE_CALYX_NATIVE_EVIDENCE_INVALID: &str =
    "CALYX_POLY_LIVE_CALYX_NATIVE_EVIDENCE_INVALID";
pub const ERR_LIVE_CALYX_NATIVE_EVIDENCE_MISSING: &str =
    "CALYX_POLY_LIVE_CALYX_NATIVE_EVIDENCE_MISSING";
pub const ERR_LIVE_CALYX_NATIVE_EVIDENCE_STALE: &str =
    "CALYX_POLY_LIVE_CALYX_NATIVE_EVIDENCE_STALE";
pub const ERR_LIVE_CALYX_NATIVE_EVIDENCE_FUTURE: &str =
    "CALYX_POLY_LIVE_CALYX_NATIVE_EVIDENCE_FUTURE";
pub const ERR_LIVE_CALYX_NATIVE_EVIDENCE_STORE: &str =
    "CALYX_POLY_LIVE_CALYX_NATIVE_EVIDENCE_STORE";
pub const ERR_LIVE_CALYX_NATIVE_EVIDENCE_READBACK: &str =
    "CALYX_POLY_LIVE_CALYX_NATIVE_EVIDENCE_READBACK";

const ACTOR: &str = "calyx-poly-live-evidence";
const FLOAT_EPSILON: f64 = 1e-6;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiveCalyxNativeEvidence {
    panel: PolyPanelSufficiencyReport,
    kernel_recall: ComputedKernelRecall,
    calibration: CalibrationRefitReport,
    goodhart: GoodhartReport,
    goodhart_held_out: HeldOutSet,
    mistake_replay: RegressionReport,
}

pub struct LiveCalyxNativeEvidenceRequest<'a> {
    pub panel: &'a PolyPanelSufficiencyReport,
    pub kernel_recall: &'a ComputedKernelRecall,
    pub calibration: &'a CalibrationRefitReport,
    pub goodhart: &'a GoodhartReport,
    pub goodhart_held_out: &'a HeldOutSet,
    pub mistake_replay: &'a RegressionReport,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredLiveCalyxNativeEvidence {
    ledger_seq: u64,
    recorded_at_millis: u64,
    payload_blake3: String,
    evidence: LiveCalyxNativeEvidence,
}

pub trait LiveCalyxNativeEvidenceStore {
    fn append_live_calyx_native_evidence(
        &self,
        subject: SubjectId,
        payload: Vec<u8>,
    ) -> calyx_core::Result<LedgerRef>;

    fn live_calyx_native_evidence_row(&self, seq: u64) -> calyx_core::Result<Option<Vec<u8>>>;

    fn scan_live_calyx_native_evidence_rows(&self) -> calyx_core::Result<Vec<Vec<u8>>>;
}

impl<C: Clock> LiveCalyxNativeEvidenceStore for AsterVault<C> {
    fn append_live_calyx_native_evidence(
        &self,
        subject: SubjectId,
        payload: Vec<u8>,
    ) -> calyx_core::Result<LedgerRef> {
        self.append_ledger_entry(
            EntryKind::Measure,
            subject,
            payload,
            ActorId::Service(ACTOR.to_string()),
        )
    }

    fn live_calyx_native_evidence_row(&self, seq: u64) -> calyx_core::Result<Option<Vec<u8>>> {
        self.read_cf_at(self.latest_seq(), ColumnFamily::Ledger, &ledger_key(seq))
    }

    fn scan_live_calyx_native_evidence_rows(&self) -> calyx_core::Result<Vec<Vec<u8>>> {
        Ok(self
            .scan_cf_at(self.latest_seq(), ColumnFamily::Ledger)?
            .into_iter()
            .map(|(_, bytes)| bytes)
            .collect())
    }
}

impl LiveCalyxNativeEvidence {
    pub fn panel(&self) -> &PolyPanelSufficiencyReport {
        &self.panel
    }

    pub fn kernel_recall(&self) -> &ComputedKernelRecall {
        &self.kernel_recall
    }

    pub fn calibration(&self) -> &CalibrationRefitReport {
        &self.calibration
    }

    pub fn panel_sufficient(&self) -> bool {
        self.panel.sufficient
            && self.panel.n_samples >= MIN_ASSAY_SAMPLES
            && self.panel.lens_count >= LIVE_CALYX_NATIVE_MIN_PANEL_LENSES
            && self.panel.panel_bits >= self.panel.anchor_entropy_bits
    }

    pub fn calibrated(&self) -> bool {
        self.calibration.observation_count >= MIN_CALIBRATION_SAMPLES
            && self.calibration.brier_improvement > 0.0
    }

    pub fn goodhart_defended(&self) -> bool {
        self.goodhart.passed
            && self.goodhart.violations.is_empty()
            && self.goodhart_held_out.sealed
            && self.goodhart_held_out.grounded_anchor_count >= MIN_ASSAY_SAMPLES
            && self
                .goodhart
                .in_region_frac
                .is_some_and(|value| value >= GOODHART_THRESHOLD as f64)
    }

    pub fn mistake_closed(&self) -> bool {
        self.mistake_replay.passed
            && self.mistake_replay.regression_count == 0
            && !self.mistake_replay.results.is_empty()
    }
}

impl StoredLiveCalyxNativeEvidence {
    pub fn evidence(&self) -> &LiveCalyxNativeEvidence {
        &self.evidence
    }

    pub fn evidence_ref(&self) -> CalyxNativeEvidenceRef {
        CalyxNativeEvidenceRef {
            ledger_seq: self.ledger_seq,
            recorded_at_millis: self.recorded_at_millis,
            panel_version: self.evidence.panel.panel_version,
            payload_blake3: self.payload_blake3.clone(),
        }
    }

    pub fn validate_for(
        &self,
        domain: &str,
        horizon_bucket: &str,
        panel_version: u32,
        forecast_at_millis: u64,
    ) -> Result<()> {
        validate_evidence(&self.evidence)?;
        if self.evidence.panel.domain != domain
            || self.evidence.calibration.slope.horizon_bucket != horizon_bucket
            || self.evidence.panel.panel_version != panel_version
        {
            return invalid(format!(
                "evidence scope {}/{}/v{} does not match forecast scope {}/{}/v{}",
                self.evidence.panel.domain,
                self.evidence.calibration.slope.horizon_bucket,
                self.evidence.panel.panel_version,
                domain,
                horizon_bucket,
                panel_version
            ));
        }
        require_not_future(
            self.recorded_at_millis,
            forecast_at_millis,
            "evidence ledger row",
        )?;
        require_fresh(
            self.recorded_at_millis,
            forecast_at_millis,
            "evidence ledger row",
        )?;
        require_not_future(
            self.evidence.calibration.as_of_millis,
            forecast_at_millis,
            "calibration refit",
        )?;
        require_fresh(
            self.evidence.calibration.as_of_millis,
            forecast_at_millis,
            "calibration refit",
        )
    }
}

pub fn record_live_calyx_native_evidence<S: LiveCalyxNativeEvidenceStore>(
    store: &S,
    request: LiveCalyxNativeEvidenceRequest<'_>,
) -> Result<StoredLiveCalyxNativeEvidence> {
    let evidence = LiveCalyxNativeEvidence {
        panel: request.panel.clone(),
        kernel_recall: request.kernel_recall.clone(),
        calibration: request.calibration.clone(),
        goodhart: request.goodhart.clone(),
        goodhart_held_out: request.goodhart_held_out.clone(),
        mistake_replay: request.mistake_replay.clone(),
    };
    validate_evidence(&evidence)?;
    let payload = encode_payload(&evidence)?;
    let subject = evidence_subject(
        &evidence.panel.domain,
        &evidence.calibration.slope.horizon_bucket,
        evidence.panel.panel_version,
    );
    let ledger_ref = store
        .append_live_calyx_native_evidence(subject.clone(), payload)
        .map_err(|error| store_error(format!("append evidence ledger row: {error}")))?;
    let row = store
        .live_calyx_native_evidence_row(ledger_ref.seq)
        .map_err(|error| {
            store_error(format!(
                "read evidence ledger row {}: {error}",
                ledger_ref.seq
            ))
        })?
        .ok_or_else(|| {
            readback_error(format!("evidence ledger row {} is absent", ledger_ref.seq))
        })?;
    let stored = decode_evidence_row(&row, &subject)?;
    if stored.ledger_seq != ledger_ref.seq || stored.evidence != evidence {
        return Err(readback_error(format!(
            "evidence ledger row {} changed during readback",
            ledger_ref.seq
        )));
    }
    Ok(stored)
}

pub fn read_latest_live_calyx_native_evidence<S: LiveCalyxNativeEvidenceStore>(
    store: &S,
    domain: &str,
    horizon_bucket: &str,
    panel_version: u32,
    forecast_at_millis: u64,
) -> Result<StoredLiveCalyxNativeEvidence> {
    let subject = evidence_subject(domain, horizon_bucket, panel_version);
    let rows = store
        .scan_live_calyx_native_evidence_rows()
        .map_err(|error| store_error(format!("scan evidence ledger rows: {error}")))?;
    let mut matching = Vec::new();
    for row in rows {
        if let Some(stored) = decode_matching_evidence_row(&row, &subject)? {
            matching.push(stored);
        }
    }
    matching.sort_by_key(|stored| stored.ledger_seq);
    if matching.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_LIVE_CALYX_NATIVE_EVIDENCE_MISSING,
            format!("no live evidence exists for {domain}/{horizon_bucket}/v{panel_version}"),
        ));
    }
    let stored = matching
        .into_iter()
        .rev()
        .find(|stored| stored.recorded_at_millis <= forecast_at_millis)
        .ok_or_else(|| {
            PolyError::diagnostics(
                ERR_LIVE_CALYX_NATIVE_EVIDENCE_FUTURE,
                format!(
                    "all live evidence for {domain}/{horizon_bucket}/v{panel_version} is after forecast timestamp {forecast_at_millis}"
                ),
            )
        })?;
    stored.validate_for(domain, horizon_bucket, panel_version, forecast_at_millis)?;
    Ok(stored)
}

fn decode_matching_evidence_row(
    bytes: &[u8],
    subject: &SubjectId,
) -> Result<Option<StoredLiveCalyxNativeEvidence>> {
    let entry = decode_ledger(bytes)
        .map_err(|error| readback_error(format!("decode Aster ledger row: {error}")))?;
    if &entry.subject != subject {
        return Ok(None);
    }
    decode_evidence_entry(entry, subject).map(Some)
}

fn decode_evidence_row(bytes: &[u8], subject: &SubjectId) -> Result<StoredLiveCalyxNativeEvidence> {
    let entry = decode_ledger(bytes)
        .map_err(|error| readback_error(format!("decode Aster ledger row: {error}")))?;
    decode_evidence_entry(entry, subject)
}

fn decode_evidence_entry(
    entry: calyx_ledger::LedgerEntry,
    subject: &SubjectId,
) -> Result<StoredLiveCalyxNativeEvidence> {
    if entry.kind != EntryKind::Measure
        || &entry.subject != subject
        || entry.actor != ActorId::Service(ACTOR.to_string())
    {
        return Err(readback_error(format!(
            "ledger row {} is not a live-evidence measurement",
            entry.seq
        )));
    }
    let evidence = decode_payload(&entry.payload, entry.seq)?;
    validate_evidence(&evidence)?;
    Ok(StoredLiveCalyxNativeEvidence {
        ledger_seq: entry.seq,
        recorded_at_millis: entry.ts,
        payload_blake3: blake3::hash(&entry.payload).to_hex().to_string(),
        evidence,
    })
}

fn evidence_subject(domain: &str, horizon_bucket: &str, panel_version: u32) -> SubjectId {
    let key =
        format!("{LIVE_CALYX_NATIVE_EVIDENCE_EVENT}\0{domain}\0{horizon_bucket}\0{panel_version}");
    SubjectId::Query(blake3::hash(key.as_bytes()).as_bytes().to_vec())
}

fn require_not_future(value: u64, forecast_at: u64, label: &str) -> Result<()> {
    if value > forecast_at {
        return Err(PolyError::diagnostics(
            ERR_LIVE_CALYX_NATIVE_EVIDENCE_FUTURE,
            format!("{label} timestamp {value} is after forecast timestamp {forecast_at}"),
        ));
    }
    Ok(())
}

fn require_fresh(value: u64, forecast_at: u64, label: &str) -> Result<()> {
    let age = forecast_at.saturating_sub(value);
    if age > LIVE_CALYX_NATIVE_EVIDENCE_MAX_AGE_MILLIS {
        return Err(PolyError::diagnostics(
            ERR_LIVE_CALYX_NATIVE_EVIDENCE_STALE,
            format!("{label} age {age}ms exceeds {LIVE_CALYX_NATIVE_EVIDENCE_MAX_AGE_MILLIS}ms"),
        ));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> Result<()> {
    Err(invalid_error(message))
}

fn invalid_error(message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(ERR_LIVE_CALYX_NATIVE_EVIDENCE_INVALID, message)
}

fn store_error(message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(ERR_LIVE_CALYX_NATIVE_EVIDENCE_STORE, message)
}

fn readback_error(message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(ERR_LIVE_CALYX_NATIVE_EVIDENCE_READBACK, message)
}
