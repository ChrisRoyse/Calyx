//! Temporal point-process co-intensity summaries (#67).
//!
//! This is a 1D event-time analogue of cross-type Ripley K: for each radius `r`,
//! count type-B events within `r` time units of each type-A event, normalize by
//! B intensity, and compare to the homogeneous independent 1D baseline `2r`.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

pub const MIN_POINT_EVENTS: usize = 2;
pub const DEFAULT_CLUSTER_RATIO: f64 = 1.5;
pub const DEFAULT_INHIBIT_RATIO: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoIntensityVerdict {
    Clustered,
    Inhibited,
    PoissonLike,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossKPoint {
    pub radius: f64,
    pub cumulative_pair_count: usize,
    pub ring_pair_count: usize,
    pub cross_k: f64,
    pub poisson_k: f64,
    pub k_ratio: f64,
    pub pair_correlation: f64,
    pub verdict: CoIntensityVerdict,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossKReport {
    pub estimator: String,
    pub edge_correction: String,
    pub observation_start: f64,
    pub observation_end: f64,
    pub duration: f64,
    pub n_a: usize,
    pub n_b: usize,
    pub lambda_b: f64,
    pub cluster_ratio: f64,
    pub inhibit_ratio: f64,
    pub strongest_cluster_radius: f64,
    pub strongest_cluster_pair_correlation: f64,
    pub strongest_inhibition_radius: f64,
    pub strongest_inhibition_pair_correlation: f64,
    pub points: Vec<CrossKPoint>,
}

pub fn temporal_cross_k(
    events_a: &[f64],
    events_b: &[f64],
    radii: &[f64],
    observation_start: f64,
    observation_end: f64,
) -> Result<CrossKReport> {
    validate_window(observation_start, observation_end)?;
    validate_events("A", events_a, observation_start, observation_end)?;
    validate_events("B", events_b, observation_start, observation_end)?;
    validate_radii(radii, observation_end - observation_start)?;

    let duration = observation_end - observation_start;
    let lambda_b = events_b.len() as f64 / duration;
    let mut points = Vec::with_capacity(radii.len());
    let mut previous_radius = 0.0;
    let mut previous_count = 0usize;

    for &radius in radii {
        let cumulative_count = count_pairs_within(events_a, events_b, radius);
        let ring_count = cumulative_count - previous_count;
        let ring_width = radius - previous_radius;
        let cross_k = cumulative_count as f64 / (events_a.len() as f64 * lambda_b);
        let poisson_k = 2.0 * radius;
        let k_ratio = cross_k / poisson_k;
        let expected_ring =
            events_a.len() as f64 * events_b.len() as f64 * (2.0 * ring_width / duration);
        let pair_correlation = ring_count as f64 / expected_ring;
        let verdict = verdict(pair_correlation);
        points.push(CrossKPoint {
            radius,
            cumulative_pair_count: cumulative_count,
            ring_pair_count: ring_count,
            cross_k,
            poisson_k,
            k_ratio,
            pair_correlation,
            verdict,
        });
        previous_radius = radius;
        previous_count = cumulative_count;
    }

    let strongest_cluster = points
        .iter()
        .max_by(|left, right| {
            left.pair_correlation
                .total_cmp(&right.pair_correlation)
                .then_with(|| right.radius.total_cmp(&left.radius))
        })
        .expect("radii are non-empty");
    let strongest_inhibition = points
        .iter()
        .min_by(|left, right| {
            left.pair_correlation
                .total_cmp(&right.pair_correlation)
                .then_with(|| left.radius.total_cmp(&right.radius))
        })
        .expect("radii are non-empty");

    Ok(CrossKReport {
        estimator: "temporal_cross_type_ripley_k".to_string(),
        edge_correction: "none; caller must choose windows/radii where boundary bias is acceptable"
            .to_string(),
        observation_start,
        observation_end,
        duration,
        n_a: events_a.len(),
        n_b: events_b.len(),
        lambda_b,
        cluster_ratio: DEFAULT_CLUSTER_RATIO,
        inhibit_ratio: DEFAULT_INHIBIT_RATIO,
        strongest_cluster_radius: strongest_cluster.radius,
        strongest_cluster_pair_correlation: strongest_cluster.pair_correlation,
        strongest_inhibition_radius: strongest_inhibition.radius,
        strongest_inhibition_pair_correlation: strongest_inhibition.pair_correlation,
        points,
    })
}

fn validate_window(start: f64, end: f64) -> Result<()> {
    if !start.is_finite() || !end.is_finite() || end <= start {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "temporal cross K requires finite observation_start < observation_end; got {start}..{end}"
        )));
    }
    Ok(())
}

fn validate_events(name: &str, events: &[f64], start: f64, end: f64) -> Result<()> {
    if events.len() < MIN_POINT_EVENTS {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "temporal cross K requires at least {MIN_POINT_EVENTS} {name} events; got {}",
            events.len()
        )));
    }
    let mut previous = None;
    for (index, &time) in events.iter().enumerate() {
        if !time.is_finite() || time < start || time > end {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "{name} event {index}={time} is not finite or outside observation window {start}..{end}"
            )));
        }
        if previous.is_some_and(|p| time <= p) {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "{name} events must be strictly increasing; index {index} has {time}"
            )));
        }
        previous = Some(time);
    }
    Ok(())
}

fn validate_radii(radii: &[f64], duration: f64) -> Result<()> {
    if radii.is_empty() {
        return Err(CalyxError::assay_insufficient_samples(
            "temporal cross K requires at least one radius",
        ));
    }
    let mut previous = 0.0;
    for (index, &radius) in radii.iter().enumerate() {
        if !radius.is_finite() || radius <= previous {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "radii must be finite and strictly increasing; radius[{index}]={radius}, previous={previous}"
            )));
        }
        if radius >= duration / 2.0 {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "radius {radius} is too large for duration {duration}; require radius < duration/2"
            )));
        }
        previous = radius;
    }
    Ok(())
}

fn count_pairs_within(events_a: &[f64], events_b: &[f64], radius: f64) -> usize {
    let mut count = 0usize;
    for &a in events_a {
        for &b in events_b {
            if (b - a).abs() <= radius {
                count += 1;
            }
        }
    }
    count
}

fn verdict(pair_correlation: f64) -> CoIntensityVerdict {
    if pair_correlation >= DEFAULT_CLUSTER_RATIO {
        CoIntensityVerdict::Clustered
    } else if pair_correlation <= DEFAULT_INHIBIT_RATIO {
        CoIntensityVerdict::Inhibited
    } else {
        CoIntensityVerdict::PoissonLike
    }
}
