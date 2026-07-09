//! Fixed-decay exponential Hawkes branching-ratio estimator (#66).
//!
//! Fits baseline rates and a target-by-source branching matrix with EM for a
//! fixed exponential kernel. This is a real Hawkes process estimator, distinct
//! from descriptive co-intensity or cross-K summaries.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_HAWKES_DECAY: f32 = 2.0;
pub const DEFAULT_HAWKES_ITERATIONS: usize = 50;
pub const DEFAULT_HAWKES_MIN_EDGE_BRANCHING_RATIO: f32 = 0.05;

#[derive(Clone, Copy)]
pub struct HawkesEventSeries<'a> {
    pub name: &'a str,
    pub event_times: &'a [f32],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HawkesConfig {
    pub observation_end: f32,
    pub decay: f32,
    pub iterations: usize,
    pub min_edge_branching_ratio: f32,
}

impl HawkesConfig {
    pub fn new(
        observation_end: f32,
        decay: f32,
        iterations: usize,
        min_edge_branching_ratio: f32,
    ) -> Self {
        Self {
            observation_end,
            decay,
            iterations,
            min_edge_branching_ratio,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HawkesStability {
    Subcritical,
    CriticalOrSupercritical,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HawkesBaseline {
    pub process: String,
    pub event_count: usize,
    pub rate: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HawkesEdge {
    pub source: String,
    pub target: String,
    pub branching_ratio: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HawkesReport {
    pub estimator: String,
    pub observation_start: f32,
    pub observation_end: f32,
    pub decay: f32,
    pub iterations: usize,
    pub processes: Vec<String>,
    pub event_counts: Vec<usize>,
    pub baseline_rates: Vec<HawkesBaseline>,
    pub branching_matrix: Vec<Vec<f32>>,
    pub retained_edges: Vec<HawkesEdge>,
    pub spectral_radius: f32,
    pub stability: HawkesStability,
}

pub fn exponential_hawkes_em(
    processes: &[HawkesEventSeries<'_>],
    config: &HawkesConfig,
) -> Result<HawkesReport> {
    validate_hawkes_inputs(processes, config)?;
    let d = processes.len();
    let observation_end = config.observation_end as f64;
    let decay = config.decay as f64;
    let events: Vec<Vec<f64>> = processes
        .iter()
        .map(|process| process.event_times.iter().map(|&t| t as f64).collect())
        .collect();
    let exposures = source_exposures(&events, observation_end, decay)?;
    let kernel_sums = previous_kernel_sums(&events, decay);
    let mut baseline_rates: Vec<f64> = events
        .iter()
        .map(|event_times| 0.5 * event_times.len() as f64 / observation_end)
        .collect();
    let mut branching_matrix = vec![vec![0.05f64; d]; d];

    for _ in 0..config.iterations {
        let mut background_counts = vec![0.0f64; d];
        let mut triggered_counts = vec![vec![0.0f64; d]; d];

        for target in 0..d {
            for source_kernel_sums in &kernel_sums[target] {
                let mut source_contributions = vec![0.0f64; d];
                let mut intensity = baseline_rates[target];
                for source in 0..d {
                    let contribution =
                        branching_matrix[target][source] * source_kernel_sums[source];
                    source_contributions[source] = contribution;
                    intensity += contribution;
                }
                if intensity <= 0.0 || !intensity.is_finite() {
                    return Err(CalyxError::assay_degenerate_input(
                        "Hawkes EM intensity became non-positive or non-finite",
                    ));
                }
                background_counts[target] += baseline_rates[target] / intensity;
                for (source, contribution) in source_contributions.iter().enumerate() {
                    triggered_counts[target][source] += contribution / intensity;
                }
            }
        }

        for target in 0..d {
            baseline_rates[target] = background_counts[target] / observation_end;
            for (source, exposure) in exposures.iter().enumerate() {
                branching_matrix[target][source] = triggered_counts[target][source] / exposure;
            }
        }
    }

    let retained_edges = retained_edges(
        processes,
        &branching_matrix,
        config.min_edge_branching_ratio,
    );
    let spectral_radius = spectral_radius(&branching_matrix) as f32;
    let stability = if spectral_radius < 1.0 {
        HawkesStability::Subcritical
    } else {
        HawkesStability::CriticalOrSupercritical
    };
    Ok(HawkesReport {
        estimator: "fixed_decay_exponential_hawkes_em".to_string(),
        observation_start: 0.0,
        observation_end: config.observation_end,
        decay: config.decay,
        iterations: config.iterations,
        processes: processes
            .iter()
            .map(|process| process.name.to_string())
            .collect(),
        event_counts: processes
            .iter()
            .map(|process| process.event_times.len())
            .collect(),
        baseline_rates: processes
            .iter()
            .enumerate()
            .map(|(idx, process)| HawkesBaseline {
                process: process.name.to_string(),
                event_count: process.event_times.len(),
                rate: baseline_rates[idx] as f32,
            })
            .collect(),
        branching_matrix: branching_matrix
            .iter()
            .map(|row| row.iter().map(|&value| value as f32).collect())
            .collect(),
        retained_edges,
        spectral_radius,
        stability,
    })
}

fn validate_hawkes_inputs(
    processes: &[HawkesEventSeries<'_>],
    config: &HawkesConfig,
) -> Result<()> {
    if processes.is_empty() {
        return Err(CalyxError::assay_insufficient_samples(
            "Hawkes EM requires at least one event process",
        ));
    }
    if !config.observation_end.is_finite() || config.observation_end <= 0.0 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "Hawkes observation_end must be finite and positive; got {}",
            config.observation_end
        )));
    }
    if !config.decay.is_finite() || config.decay <= 0.0 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "Hawkes decay must be finite and positive; got {}",
            config.decay
        )));
    }
    if config.iterations == 0 || config.iterations > 1_000 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "Hawkes iterations must be in 1..=1000; got {}",
            config.iterations
        )));
    }
    if !config.min_edge_branching_ratio.is_finite() || config.min_edge_branching_ratio < 0.0 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "Hawkes min_edge_branching_ratio must be finite and non-negative; got {}",
            config.min_edge_branching_ratio
        )));
    }

    let mut names = std::collections::BTreeSet::new();
    for process in processes {
        if process.name.trim().is_empty() || !names.insert(process.name) {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "Hawkes process names must be non-empty and unique; bad name {:?}",
                process.name
            )));
        }
        if process.event_times.len() < 2 {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "Hawkes process {} requires at least two events; got {}",
                process.name,
                process.event_times.len()
            )));
        }
        let mut previous = None;
        for (idx, &event_time) in process.event_times.iter().enumerate() {
            if !event_time.is_finite() || event_time < 0.0 || event_time >= config.observation_end {
                return Err(CalyxError::assay_insufficient_samples(format!(
                    "Hawkes {} event[{idx}] must be finite in [0, observation_end); got {event_time}",
                    process.name
                )));
            }
            if let Some(prev) = previous
                && event_time <= prev
            {
                return Err(CalyxError::assay_insufficient_samples(format!(
                    "Hawkes {} events must be strictly increasing; event[{idx}]={event_time} after {prev}",
                    process.name
                )));
            }
            previous = Some(event_time);
        }
    }
    Ok(())
}

fn source_exposures(events: &[Vec<f64>], observation_end: f64, decay: f64) -> Result<Vec<f64>> {
    let mut exposures = Vec::with_capacity(events.len());
    for event_times in events {
        let exposure = event_times
            .iter()
            .map(|&event_time| 1.0 - (-decay * (observation_end - event_time)).exp())
            .sum::<f64>();
        if exposure <= 0.0 || !exposure.is_finite() {
            return Err(CalyxError::assay_degenerate_input(
                "Hawkes source exposure is non-positive or non-finite",
            ));
        }
        exposures.push(exposure);
    }
    Ok(exposures)
}

fn previous_kernel_sum(event_times: &[f64], target_time: f64, decay: f64) -> f64 {
    let mut sum = 0.0f64;
    for &source_time in event_times {
        if source_time >= target_time {
            break;
        }
        sum += decay * (-decay * (target_time - source_time)).exp();
    }
    sum
}

fn previous_kernel_sums(events: &[Vec<f64>], decay: f64) -> Vec<Vec<Vec<f64>>> {
    let d = events.len();
    let mut sums = Vec::with_capacity(d);
    for target in 0..d {
        let mut target_sums = Vec::with_capacity(events[target].len());
        for &event_time in &events[target] {
            target_sums.push(
                (0..d)
                    .map(|source| previous_kernel_sum(&events[source], event_time, decay))
                    .collect(),
            );
        }
        sums.push(target_sums);
    }
    sums
}

fn retained_edges(
    processes: &[HawkesEventSeries<'_>],
    branching_matrix: &[Vec<f64>],
    min_edge_branching_ratio: f32,
) -> Vec<HawkesEdge> {
    let threshold = min_edge_branching_ratio as f64;
    let mut edges = Vec::new();
    for (target, row) in branching_matrix.iter().enumerate() {
        for (source, &branching_ratio) in row.iter().enumerate() {
            if branching_ratio >= threshold {
                edges.push(HawkesEdge {
                    source: processes[source].name.to_string(),
                    target: processes[target].name.to_string(),
                    branching_ratio: branching_ratio as f32,
                });
            }
        }
    }
    edges
}

fn spectral_radius(matrix: &[Vec<f64>]) -> f64 {
    let d = matrix.len();
    let mut vector = vec![1.0 / d as f64; d];
    let mut eigenvalue = 0.0f64;
    for _ in 0..100 {
        let mut next = vec![0.0f64; d];
        for row in 0..d {
            for (col, value) in vector.iter().enumerate() {
                next[row] += matrix[row][col] * value;
            }
        }
        let norm = next.iter().sum::<f64>();
        if norm <= 1e-15 || !norm.is_finite() {
            return 0.0;
        }
        for value in &mut next {
            *value /= norm;
        }
        eigenvalue = norm;
        vector = next;
    }
    eigenvalue
}
