use super::*;

pub(super) fn temporal_recent(row: &ChunkRow, context: TemporalContext) -> Result<SlotVector> {
    let Some(input) = event_time_input(row)? else {
        return Ok(temporal_absent());
    };
    E2RecencyLens::new(E2RecencyConfig {
        decay: DecayFunction::Linear {
            max_age_secs: context.max_age_secs,
        },
        reference_time: context.reference_time,
    })
    .measure(&input)
}

pub(super) fn temporal_periodic(row: &ChunkRow, context: TemporalContext) -> Result<SlotVector> {
    let Some(input) = event_time_input(row)? else {
        return Ok(temporal_absent());
    };
    E3PeriodicLens::new(E3PeriodicConfig {
        options: PeriodicOptions {
            target_hour: None,
            target_day_of_week: None,
            use_now: true,
        },
        reference_time: context.reference_time,
    })
    .measure(&input)
}

pub(super) fn temporal_positional(
    row: &ChunkRow,
    position: u64,
    context: TemporalContext,
) -> Result<SlotVector> {
    if row.event_time_secs.is_none() {
        return Ok(temporal_absent());
    }
    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(&position.to_le_bytes());
    bytes.extend_from_slice(&context.total_position.to_le_bytes());
    E4PositionalLens::new(E4PositionalConfig {
        options: SequenceOptions::default(),
    })
    .measure(&Input::new(Modality::Structured, bytes))
}

fn event_time_input(row: &ChunkRow) -> Result<Option<Input>> {
    let Some(secs) = row.event_time_secs else {
        return Ok(None);
    };
    let timestamp = i64::try_from(secs).map_err(|_| {
        errors::backfill_incomplete(format!(
            "row {} source event timestamp {secs} exceeds i64",
            row.row_num
        ))
    })?;
    Ok(Some(Input::new(
        Modality::Structured,
        timestamp.to_le_bytes().to_vec(),
    )))
}

fn temporal_absent() -> SlotVector {
    SlotVector::Absent {
        reason: AbsentReason::Error(TEMPORAL_MISSING_CREATED_AT.to_string()),
    }
}
