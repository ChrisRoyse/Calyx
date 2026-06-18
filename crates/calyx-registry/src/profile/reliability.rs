use calyx_assay::contract::MIN_RELIABILITY_SEEDS;
use calyx_core::{CalyxError, Result};

use super::CapabilityCard;

pub(crate) fn signal_floor(card: &CapabilityCard) -> Result<(f32, Option<String>)> {
    let Some(reliability) = &card.signal_reliability else {
        return Ok((card.signal.unwrap_or(0.0), None));
    };
    for (name, value) in [
        ("ci_low", reliability.ci_low),
        ("ci_high", reliability.ci_high),
        ("seed_sigma", reliability.seed_sigma),
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(CalyxError::assay_low_signal(format!(
                "capability signal reliability {name} must be finite and non-negative"
            )));
        }
    }
    if reliability.seed_count < MIN_RELIABILITY_SEEDS {
        return Ok((
            reliability.ci_low,
            Some(format!(
                "assay reliability used {} seeds; need at least {MIN_RELIABILITY_SEEDS}",
                reliability.seed_count
            )),
        ));
    }
    if reliability.unresolved {
        return Ok((
            reliability.ci_low,
            Some(format!(
                "assay reliability unresolved: ci=[{:.4},{:.4}] seed_sigma={:.4}",
                reliability.ci_low, reliability.ci_high, reliability.seed_sigma
            )),
        ));
    }
    Ok((reliability.ci_low, None))
}
