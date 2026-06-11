use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, Result};

use crate::CALYX_ASTER_CF_UNAVAILABLE;

use super::types::{
    CALYX_ANNEAL_SOAK_INVALID_ROW, MetricSample, SoakReport, SoakRowKind, SoakStoredRow,
};

const SOAK_ROW_TAG: &str = "anneal_soak_v1";
const SOAK_REPORT_PREFIX: &[u8] = b"soak_report\0";
const SOAK_SAMPLE_PREFIX: &[u8] = b"soak_sample\0";

pub trait SoakStorage {
    fn save_sample(&mut self, run_id: [u8; 32], sample: &MetricSample) -> Result<()>;
    fn save_report(&mut self, run_id: [u8; 32], report: &SoakReport) -> Result<()>;
    fn scan_rows(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;
}

#[derive(Default)]
pub struct NoopSoakStorage;

impl SoakStorage for NoopSoakStorage {
    fn save_sample(&mut self, _run_id: [u8; 32], _sample: &MetricSample) -> Result<()> {
        Ok(())
    }

    fn save_report(&mut self, _run_id: [u8; 32], _report: &SoakReport) -> Result<()> {
        Ok(())
    }

    fn scan_rows(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(Vec::new())
    }
}

pub struct AsterSoakStorage<'a, C>
where
    C: Clock,
{
    vault: &'a AsterVault<C>,
}

impl<'a, C> AsterSoakStorage<'a, C>
where
    C: Clock,
{
    pub const fn new(vault: &'a AsterVault<C>) -> Self {
        Self { vault }
    }
}

impl<C> SoakStorage for AsterSoakStorage<'_, C>
where
    C: Clock,
{
    fn save_sample(&mut self, run_id: [u8; 32], sample: &MetricSample) -> Result<()> {
        self.vault
            .write_cf(
                ColumnFamily::AnnealSoak,
                soak_sample_key(run_id, sample.query_count),
                encode_soak_row(run_id, SoakRowKind::Sample { sample: *sample })?,
            )
            .map(|_| ())
            .map_err(|error| cf_unavailable("write anneal_soak sample", error))
    }

    fn save_report(&mut self, run_id: [u8; 32], report: &SoakReport) -> Result<()> {
        self.vault
            .write_cf(
                ColumnFamily::AnnealSoak,
                soak_report_key(run_id, report.total_queries),
                encode_soak_row(
                    run_id,
                    SoakRowKind::Report {
                        report: report.clone(),
                    },
                )?,
            )
            .map(|_| ())
            .map_err(|error| cf_unavailable("write anneal_soak report", error))
    }

    fn scan_rows(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.vault
            .scan_cf_at(self.vault.latest_seq(), ColumnFamily::AnnealSoak)
            .map_err(|error| cf_unavailable("scan anneal_soak CF", error))
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SoakRowEnvelope {
    tag: String,
    row: SoakStoredRow,
}

pub fn soak_report_key(run_id: [u8; 32], total_queries: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(SOAK_REPORT_PREFIX.len() + run_id.len() + 8);
    key.extend_from_slice(SOAK_REPORT_PREFIX);
    key.extend_from_slice(&run_id);
    key.extend_from_slice(&total_queries.to_be_bytes());
    key
}

pub fn soak_sample_key(run_id: [u8; 32], query_count: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(SOAK_SAMPLE_PREFIX.len() + run_id.len() + 8);
    key.extend_from_slice(SOAK_SAMPLE_PREFIX);
    key.extend_from_slice(&run_id);
    key.extend_from_slice(&query_count.to_be_bytes());
    key
}

pub fn encode_soak_row(run_id: [u8; 32], row: SoakRowKind) -> Result<Vec<u8>> {
    let envelope = SoakRowEnvelope {
        tag: SOAK_ROW_TAG.to_string(),
        row: SoakStoredRow { run_id, row },
    };
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(&envelope, &mut bytes)
        .map_err(|error| invalid_row(error.to_string()))?;
    Ok(bytes)
}

pub fn decode_soak_row(bytes: &[u8]) -> Result<SoakStoredRow> {
    let envelope: SoakRowEnvelope =
        ciborium::de::from_reader(bytes).map_err(|error| invalid_row(error.to_string()))?;
    if envelope.tag != SOAK_ROW_TAG {
        return Err(invalid_row(format!(
            "unexpected anneal_soak row tag {}",
            envelope.tag
        )));
    }
    Ok(envelope.row)
}

pub fn decode_soak_reports(rows: &[(Vec<u8>, Vec<u8>)]) -> Result<Vec<SoakReport>> {
    let mut reports = Vec::new();
    for (_key, value) in rows {
        let row = decode_soak_row(value)?;
        if let SoakRowKind::Report { report } = row.row {
            reports.push(report);
        }
    }
    reports.sort_by_key(|report| (report.ts, report.total_queries));
    Ok(reports)
}

fn invalid_row(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_SOAK_INVALID_ROW,
        message: message.into(),
        remediation: "repair or quarantine anneal_soak CF rows before reading soak reports",
    }
}

fn cf_unavailable(context: &'static str, error: CalyxError) -> CalyxError {
    CalyxError {
        code: CALYX_ASTER_CF_UNAVAILABLE,
        message: format!("{context}: {}: {}", error.code, error.message),
        remediation: "restore Aster anneal_soak CF availability",
    }
}
