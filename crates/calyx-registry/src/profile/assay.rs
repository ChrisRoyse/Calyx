use calyx_assay::store::{AssayCacheKey, AssayStore, AssaySubject};
use calyx_core::{Result, Slot, SlotId};

use super::{
    CapabilityCard, CapabilitySignalReliability, MetricSource, ProfileProbe, profile_lens,
};
use crate::lens::Registry;

pub fn profile_slot_with_assay(
    registry: &Registry,
    slot: &Slot,
    probes: &[ProfileProbe],
    assay_store: &AssayStore,
    cache_key: &AssayCacheKey,
) -> Result<CapabilityCard> {
    let mut card = profile_lens(registry, slot.lens_id, probes)?;
    apply_assay_metrics(&mut card, slot.slot_id, assay_store, cache_key);
    Ok(card)
}

pub fn apply_assay_metrics(
    card: &mut CapabilityCard,
    slot: SlotId,
    assay_store: &AssayStore,
    cache_key: &AssayCacheKey,
) {
    if let Some(row) = assay_store.get(cache_key, &AssaySubject::Lens { slot }) {
        card.signal = Some(row.estimate.bits);
        card.signal_source = MetricSource::AssayStore;
        card.signal_reliability =
            row.estimate
                .reliability
                .as_ref()
                .map(|reliability| CapabilitySignalReliability {
                    ci_low: row.estimate.ci_low,
                    ci_high: row.estimate.ci_high,
                    seed_sigma: reliability.seed_sigma,
                    seed_count: reliability.seed_count,
                    unresolved: reliability.unresolved,
                });
    }
    if let Some(bits) = max_pair_gain_bits(slot, assay_store, cache_key) {
        card.differentiation = Some(bits);
        card.differentiation_source = MetricSource::AssayStore;
    }
}

fn max_pair_gain_bits(
    slot: SlotId,
    assay_store: &AssayStore,
    cache_key: &AssayCacheKey,
) -> Option<f32> {
    assay_store
        .rows()
        .into_iter()
        .filter(|row| row.cache_key == *cache_key)
        .filter_map(|row| match row.subject {
            AssaySubject::Pair { a, b } if a == slot || b == slot => Some(row.estimate.bits),
            _ => None,
        })
        .max_by(|left, right| left.total_cmp(right))
}
