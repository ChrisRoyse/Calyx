use std::collections::VecDeque;
use std::sync::Arc;

use calyx_aster::{cf::ColumnFamily, vault::AsterVault};
use calyx_core::{CalyxError, Clock, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{IntelligenceReport, LogicalTime, ReportAvailability};

pub const ANNEAL_GROWTH_TAG: &str = "calyx_anneal_growth_v1";
pub const DEFAULT_GROWTH_MAX_SAMPLES: usize = 10_000;
pub const DEFAULT_GROWTH_WINDOW: usize = 10;
pub const CALYX_ANNEAL_GROWTH_INVALID_CONFIG: &str = "CALYX_ANNEAL_GROWTH_INVALID_CONFIG";
pub const CALYX_ANNEAL_GROWTH_INVALID_ROW: &str = "CALYX_ANNEAL_GROWTH_INVALID_ROW";
pub const CALYX_ANNEAL_GROWTH_INVALID_SAMPLE: &str = "CALYX_ANNEAL_GROWTH_INVALID_SAMPLE";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GrowthSample {
    pub ts: LogicalTime,
    pub j: f64,
    pub delta_j: f64,
    pub n_queries_since_last: u64,
    pub actions_taken: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GrowthSummary {
    pub samples_count: usize,
    pub j_first: Option<f64>,
    pub j_last: Option<f64>,
    pub j_max: Option<f64>,
    pub slope_recent: Option<f64>,
    pub is_rising: bool,
}

pub trait GrowthCf: Send + Sync {
    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<()>;
    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;
}

pub struct AsterGrowthCf<'a, C>
where
    C: Clock,
{
    vault: &'a AsterVault<C>,
}

impl<'a, C> AsterGrowthCf<'a, C>
where
    C: Clock,
{
    pub const fn new(vault: &'a AsterVault<C>) -> Self {
        Self { vault }
    }
}

impl<C> GrowthCf for AsterGrowthCf<'_, C>
where
    C: Clock,
{
    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        self.vault
            .write_cf(ColumnFamily::AnnealGrowth, key, value)?;
        self.vault.flush()
    }

    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.vault
            .scan_cf_at(self.vault.latest_seq(), ColumnFamily::AnnealGrowth)
    }
}

pub struct GrowthCurve<S>
where
    S: GrowthCf,
{
    samples: VecDeque<GrowthSample>,
    max_samples: usize,
    cf: S,
    clock: Arc<dyn Clock>,
    next_seq: u64,
}

impl<S> GrowthCurve<S>
where
    S: GrowthCf,
{
    pub fn load_from_cf(cf: S, clock: Arc<dyn Clock>, max_samples: usize) -> Result<Self> {
        if max_samples == 0 {
            return Err(invalid_config("max_samples must be positive"));
        }
        let mut rows = cf
            .scan()?
            .into_iter()
            .map(|(key, value)| {
                let (seq, sample) = decode_growth_row(&value)?;
                let expected = anneal_growth_key(sample.ts, seq);
                if key != expected {
                    return Err(invalid_row(format!(
                        "growth row key {} does not match ts {} seq {}",
                        hex_bytes(&key),
                        sample.ts,
                        seq
                    )));
                }
                Ok((key, seq, sample))
            })
            .collect::<Result<Vec<_>>>()?;
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        let next_seq = rows
            .iter()
            .map(|(_, seq, _)| *seq)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut samples = rows
            .into_iter()
            .map(|(_, _, sample)| sample)
            .collect::<VecDeque<_>>();
        trim_samples(&mut samples, max_samples);
        Ok(Self {
            samples,
            max_samples,
            cf,
            clock,
            next_seq,
        })
    }

    pub fn new(cf: S, clock: Arc<dyn Clock>) -> Result<Self> {
        Self::load_from_cf(cf, clock, DEFAULT_GROWTH_MAX_SAMPLES)
    }

    pub fn record_sample(
        &mut self,
        report: &IntelligenceReport,
        n_queries_since_last: u64,
        actions_taken: Vec<String>,
    ) -> Result<GrowthSample> {
        validate_report(report)?;
        let delta_j = self.samples.back().map_or(0.0, |last| report.j - last.j);
        let sample = GrowthSample {
            ts: self.clock.now(),
            j: report.j,
            delta_j,
            n_queries_since_last,
            actions_taken,
        };
        let seq = self.next_seq;
        self.next_seq = self
            .next_seq
            .checked_add(1)
            .ok_or_else(|| invalid_config("growth sample sequence exhausted"))?;
        self.push_sample(sample.clone());
        self.cf.put(
            anneal_growth_key(sample.ts, seq),
            encode_growth_row(seq, &sample)?,
        )?;
        Ok(sample)
    }

    pub fn is_rising(&self, window: usize) -> bool {
        let Some(slope) = self.slope_recent(window) else {
            return false;
        };
        let latest_delta = self.samples.back().map_or(0.0, |sample| sample.delta_j);
        slope > 0.0 && latest_delta > 0.0
    }

    pub fn curve_summary(&self) -> GrowthSummary {
        self.curve_summary_with_window(DEFAULT_GROWTH_WINDOW)
    }

    pub fn curve_summary_with_window(&self, window: usize) -> GrowthSummary {
        GrowthSummary {
            samples_count: self.samples.len(),
            j_first: self.samples.front().map(|sample| sample.j),
            j_last: self.samples.back().map(|sample| sample.j),
            j_max: self.samples.iter().map(|sample| sample.j).reduce(f64::max),
            slope_recent: self.slope_recent(window),
            is_rising: self.is_rising(window),
        }
    }

    pub fn plot_ascii(&self, width: usize, height: usize) -> String {
        if self.samples.is_empty() || width == 0 || height == 0 {
            return String::new();
        }
        let cols = width.min(self.samples.len()).max(1);
        let values = resample_values(&self.samples, cols);
        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let mut grid = vec![vec![' '; cols]; height];
        for (col, value) in values.iter().enumerate() {
            let row = plot_row(*value, min, max, height);
            grid[row][col] = '*';
        }
        grid.into_iter()
            .map(|row| row.into_iter().collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn samples(&self) -> impl DoubleEndedIterator<Item = &GrowthSample> {
        self.samples.iter()
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    fn slope_recent(&self, window: usize) -> Option<f64> {
        let n = window.min(self.samples.len());
        if n < 2 {
            return None;
        }
        let start = self.samples.len() - n;
        let ys = self
            .samples
            .iter()
            .skip(start)
            .map(|sample| sample.j)
            .collect::<Vec<_>>();
        Some(linear_slope(&ys))
    }

    fn push_sample(&mut self, sample: GrowthSample) {
        self.samples.push_back(sample);
        trim_samples(&mut self.samples, self.max_samples);
    }
}

pub fn anneal_growth_key(ts: LogicalTime, seq: u64) -> Vec<u8> {
    [ts.to_be_bytes(), seq.to_be_bytes()].concat()
}

pub fn encode_growth_row(seq: u64, sample: &GrowthSample) -> Result<Vec<u8>> {
    let row = json!({
        "tag": ANNEAL_GROWTH_TAG,
        "seq": seq,
        "sample": sample,
    });
    serde_json::to_vec_pretty(&row)
        .map_err(|error| invalid_row(format!("encode growth row JSON: {error}")))
}

pub fn decode_growth_row(bytes: &[u8]) -> Result<(u64, GrowthSample)> {
    let row: Value = serde_json::from_slice(bytes)
        .map_err(|error| invalid_row(format!("parse growth row JSON: {error}")))?;
    match row.get("tag").and_then(Value::as_str) {
        Some(ANNEAL_GROWTH_TAG) => {}
        Some(other) => return Err(invalid_row(format!("unexpected growth row tag {other}"))),
        None => return Err(invalid_row("growth row missing tag")),
    }
    let seq = row
        .get("seq")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_row("growth row seq must be u64"))?;
    let sample = row
        .get("sample")
        .cloned()
        .ok_or_else(|| invalid_row("growth row missing sample"))
        .and_then(|value| {
            serde_json::from_value::<GrowthSample>(value)
                .map_err(|error| invalid_row(format!("decode growth sample: {error}")))
        })?;
    validate_sample(&sample)?;
    Ok((seq, sample))
}

fn validate_report(report: &IntelligenceReport) -> Result<()> {
    if !matches!(&report.availability, ReportAvailability::Available) {
        return Err(invalid_sample(
            "cannot record unavailable intelligence report in growth curve",
        ));
    }
    if !report.j.is_finite() {
        return Err(invalid_sample(format!(
            "growth sample J must be finite, got {}",
            report.j
        )));
    }
    Ok(())
}

fn validate_sample(sample: &GrowthSample) -> Result<()> {
    if !sample.j.is_finite() || !sample.delta_j.is_finite() {
        return Err(invalid_row(
            "growth sample j and delta_j must be finite numeric values",
        ));
    }
    Ok(())
}

fn trim_samples(samples: &mut VecDeque<GrowthSample>, max_samples: usize) {
    while samples.len() > max_samples {
        samples.pop_front();
    }
}

fn linear_slope(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    let sum_x = (0..values.len()).map(|index| index as f64).sum::<f64>();
    let sum_y = values.iter().sum::<f64>();
    let sum_x2 = (0..values.len())
        .map(|index| {
            let x = index as f64;
            x * x
        })
        .sum::<f64>();
    let sum_xy = values
        .iter()
        .enumerate()
        .map(|(index, value)| index as f64 * value)
        .sum::<f64>();
    let denominator = n * sum_x2 - sum_x * sum_x;
    if denominator == 0.0 {
        0.0
    } else {
        (n * sum_xy - sum_x * sum_y) / denominator
    }
}

fn resample_values(samples: &VecDeque<GrowthSample>, cols: usize) -> Vec<f64> {
    if cols == 1 {
        return vec![samples.back().expect("non-empty samples").j];
    }
    let last = samples.len() - 1;
    (0..cols)
        .map(|col| {
            let index = col * last / (cols - 1);
            samples[index].j
        })
        .collect()
}

fn plot_row(value: f64, min: f64, max: f64, height: usize) -> usize {
    if height == 1 || (max - min).abs() < f64::EPSILON {
        return height / 2;
    }
    let normalized = ((value - min) / (max - min)).clamp(0.0, 1.0);
    (height - 1).saturating_sub((normalized * (height - 1) as f64).round() as usize)
}

fn invalid_config(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_GROWTH_INVALID_CONFIG,
        message: message.into(),
        remediation: "configure a positive growth-curve sample window before recording J",
    }
}

fn invalid_row(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_GROWTH_INVALID_ROW,
        message: message.into(),
        remediation: "quarantine corrupt anneal_growth rows and rebuild from reports",
    }
}

fn invalid_sample(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_GROWTH_INVALID_SAMPLE,
        message: message.into(),
        remediation: "record only available finite intelligence reports in the growth curve",
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
