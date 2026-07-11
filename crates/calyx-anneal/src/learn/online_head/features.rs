use calyx_core::{CalyxError, Constellation, CxId, Result};

use super::CALYX_ANNEAL_HEAD_FEATURE_SOURCE_UNAVAILABLE;
use crate::learn::{RegressionContextSource, ReplayEntry};

pub(crate) fn resolve_replay_contexts<C>(
    batch: &[ReplayEntry],
    source: &C,
) -> Result<Vec<Constellation>>
where
    C: RegressionContextSource,
{
    batch
        .iter()
        .map(|entry| {
            let context = source
                .regression_constellation(entry.cx_id)
                .map_err(|error| feature_source_unavailable(entry.cx_id, error.to_string()))?;
            if context.cx_id != entry.cx_id {
                return Err(feature_source_unavailable(
                    entry.cx_id,
                    format!("source returned constellation {}", context.cx_id),
                ));
            }
            Ok(context)
        })
        .collect()
}

pub(crate) fn constellation_features(cx: &Constellation, len: usize) -> Vec<f32> {
    let mut features = Vec::with_capacity(len);
    if len == 0 {
        return features;
    }
    features.push(1.0);
    features.extend(
        cx.scalars
            .values()
            .take(len.saturating_sub(features.len()))
            .map(|value| *value as f32),
    );
    for value in cx
        .slots
        .values()
        .filter_map(|slot| slot.as_dense())
        .flatten()
    {
        if features.len() == len {
            break;
        }
        features.push(*value);
    }
    features.resize(len, 0.0);
    features
}

fn feature_source_unavailable(cx_id: CxId, message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_HEAD_FEATURE_SOURCE_UNAVAILABLE,
        message: format!(
            "predictor feature source unavailable for {cx_id}: {}",
            message.into()
        ),
        remediation: "restore the exact replay constellation before updating the predictor head",
    }
}
