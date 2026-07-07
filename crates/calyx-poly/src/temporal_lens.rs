//! Calyx E2/E3/E4 temporal retrieval sidecar lenses for Poly (#43).

use calyx_core::{AbsentReason, Input, Lens, Modality, SlotId, SlotShape, SlotVector};
use calyx_registry::{
    DecayFunction, E2RecencyConfig, E2RecencyLens, E3PeriodicConfig, E3PeriodicLens,
    E4PositionalConfig, E4PositionalLens, PeriodicOptions, SequenceOptions,
};

use crate::lenses::SignalLens;
use crate::model::MarketSnapshot;

pub const E2_RECENCY_KEY: &str = "E2_recency";
pub const E3_PERIODIC_KEY: &str = "E3_periodic";
pub const E4_POSITIONAL_KEY: &str = "E4_positional";
pub const ERR_TEMPORAL_INVALID: &str = "CALYX_POLY_TEMPORAL_INVALID";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemporalLensKind {
    E2Recency,
    E3Periodic,
    E4Positional,
}

pub struct PolyTemporalLens {
    slot: SlotId,
    key: String,
    kind: TemporalLensKind,
}

impl PolyTemporalLens {
    pub fn new(slot: u16, key: impl Into<String>, kind: TemporalLensKind) -> Self {
        Self {
            slot: SlotId::new(slot),
            key: key.into(),
            kind,
        }
    }
}

impl SignalLens for PolyTemporalLens {
    fn slot(&self) -> SlotId {
        self.slot
    }

    fn key(&self) -> &str {
        &self.key
    }

    fn shape(&self) -> SlotShape {
        temporal_shape(self.kind)
    }

    fn measure(&self, snapshot: &MarketSnapshot) -> SlotVector {
        match compute_temporal_vector(snapshot, self.kind) {
            Ok(data) => SlotVector::Dense {
                dim: data.len() as u32,
                data,
            },
            Err(reason) => SlotVector::Absent { reason },
        }
    }
}

pub fn compute_temporal_vector(
    snapshot: &MarketSnapshot,
    kind: TemporalLensKind,
) -> std::result::Result<Vec<f32>, AbsentReason> {
    match kind {
        TemporalLensKind::E2Recency => measure_e2(snapshot),
        TemporalLensKind::E3Periodic => measure_e3(snapshot),
        TemporalLensKind::E4Positional => measure_e4(snapshot),
    }
}

pub fn temporal_shape(kind: TemporalLensKind) -> SlotShape {
    match kind {
        TemporalLensKind::E2Recency => SlotShape::Dense(1),
        TemporalLensKind::E3Periodic => SlotShape::Dense(2),
        TemporalLensKind::E4Positional => SlotShape::Dense(4),
    }
}

pub fn is_temporal_lens_key(key: &str) -> bool {
    matches!(key, E2_RECENCY_KEY | E3_PERIODIC_KEY | E4_POSITIONAL_KEY)
}

fn measure_e2(snapshot: &MarketSnapshot) -> std::result::Result<Vec<f32>, AbsentReason> {
    let event_ts = checked_i64(snapshot.snapshot_ts, "snapshot_ts")?;
    let reference_time = temporal_reference(snapshot)?;
    let lens = E2RecencyLens::new(E2RecencyConfig {
        decay: DecayFunction::Exponential {
            half_life_secs: 86_400,
        },
        reference_time,
    });
    dense_from_slot(
        lens.measure(&timestamp_input(event_ts))
            .map_err(absent_error)?,
    )
}

fn measure_e3(snapshot: &MarketSnapshot) -> std::result::Result<Vec<f32>, AbsentReason> {
    let event_ts = checked_i64(snapshot.snapshot_ts, "snapshot_ts")?;
    let reference_time = temporal_reference(snapshot)?;
    let lens = E3PeriodicLens::new(E3PeriodicConfig {
        options: PeriodicOptions {
            use_now: true,
            ..PeriodicOptions::default()
        },
        reference_time,
    });
    dense_from_slot(
        lens.measure(&timestamp_input(event_ts))
            .map_err(absent_error)?,
    )
}

fn measure_e4(snapshot: &MarketSnapshot) -> std::result::Result<Vec<f32>, AbsentReason> {
    let position = snapshot
        .sequence_position
        .ok_or(AbsentReason::LensUnavailable)?;
    let total = snapshot
        .sequence_total
        .ok_or(AbsentReason::LensUnavailable)?;
    if total == 0 || position > total {
        return Err(AbsentReason::Error(format!(
            "{ERR_TEMPORAL_INVALID}: sequence_position={position} sequence_total={total}"
        )));
    }
    let lens = E4PositionalLens::new(E4PositionalConfig {
        options: SequenceOptions::default(),
    });
    dense_from_slot(
        lens.measure(&position_input(position, total))
            .map_err(absent_error)?,
    )
}

fn temporal_reference(snapshot: &MarketSnapshot) -> std::result::Result<i64, AbsentReason> {
    let reference = snapshot
        .temporal_reference_ts
        .ok_or(AbsentReason::LensUnavailable)?;
    if reference < snapshot.snapshot_ts {
        return Err(AbsentReason::Error(format!(
            "{ERR_TEMPORAL_INVALID}: temporal_reference_ts={reference} precedes snapshot_ts={}",
            snapshot.snapshot_ts
        )));
    }
    checked_i64(reference, "temporal_reference_ts")
}

fn checked_i64(value: u64, field: &str) -> std::result::Result<i64, AbsentReason> {
    i64::try_from(value).map_err(|_| {
        AbsentReason::Error(format!(
            "{ERR_TEMPORAL_INVALID}: {field}={value} exceeds i64::MAX"
        ))
    })
}

fn timestamp_input(timestamp: i64) -> Input {
    Input::new(Modality::Structured, timestamp.to_le_bytes().to_vec())
}

fn position_input(position: u64, total: u64) -> Input {
    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(&position.to_le_bytes());
    bytes.extend_from_slice(&total.to_le_bytes());
    Input::new(Modality::Structured, bytes)
}

fn dense_from_slot(slot: SlotVector) -> std::result::Result<Vec<f32>, AbsentReason> {
    match slot {
        SlotVector::Dense { data, .. } => Ok(data),
        other => Err(AbsentReason::Error(format!(
            "{ERR_TEMPORAL_INVALID}: Calyx temporal lens returned non-dense vector {other:?}"
        ))),
    }
}

fn absent_error(err: calyx_core::CalyxError) -> AbsentReason {
    AbsentReason::Error(format!("{}: {}", err.code, err.message))
}
