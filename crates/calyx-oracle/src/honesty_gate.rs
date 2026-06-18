//! Oracle honesty gate backed by Assay sufficiency rows.

use std::collections::BTreeMap;

use calyx_assay::{
    AssayCacheKey, AssayRow, AssayStore, AssaySubject, DeficitRoutingContext, PanelSufficiency,
    TrustTag, panel_sufficiency_with_context, per_sensor_attribution,
};
use calyx_aster::vault::AsterVault;
use calyx_core::{AnchorKind, CalyxError, Clock, LensId, Panel, SlotId};

use crate::{DomainId, OracleError, SufficiencyBound};

const SOLE_CARRIER_BITS: f32 = 0.10;

pub trait SufficiencyAssay {
    fn panel_sufficiency(
        &self,
        panel: &Panel,
        domain: &DomainId,
        clock: &dyn Clock,
    ) -> Result<PanelSufficiency, OracleError>;
}

pub fn check_sufficiency<C>(
    vault: &AsterVault<C>,
    panel: &Panel,
    domain: DomainId,
    clock: &dyn Clock,
) -> Result<SufficiencyBound, OracleError>
where
    C: Clock,
{
    let assay = VaultSufficiencyAssay::new(vault);
    check_sufficiency_with_assay(&assay, panel, domain, clock)
}

pub fn check_sufficiency_with_assay<A>(
    assay: &A,
    panel: &Panel,
    domain: DomainId,
    clock: &dyn Clock,
) -> Result<SufficiencyBound, OracleError>
where
    A: SufficiencyAssay,
{
    let report = assay.panel_sufficiency(panel, &domain, clock)?;
    validate_report(&report)?;
    let sufficient = report.panel_bits >= report.anchor_entropy_bits;
    let per_sensor_deficit = if sufficient {
        Vec::new()
    } else {
        lens_deficits(panel, &report)?
    };
    let bound = SufficiencyBound {
        i_panel_oracle: report.panel_bits,
        dpi_ceiling: report.panel_bits,
        sufficient,
        per_sensor_deficit,
    };

    if sufficient {
        Ok(bound)
    } else {
        Err(OracleError::Insufficient { bound })
    }
}

pub struct VaultSufficiencyAssay<'a, C>
where
    C: Clock,
{
    vault: &'a AsterVault<C>,
}

impl<'a, C> VaultSufficiencyAssay<'a, C>
where
    C: Clock,
{
    pub const fn new(vault: &'a AsterVault<C>) -> Self {
        Self { vault }
    }
}

impl<C> SufficiencyAssay for VaultSufficiencyAssay<'_, C>
where
    C: Clock,
{
    fn panel_sufficiency(
        &self,
        panel: &Panel,
        domain: &DomainId,
        clock: &dyn Clock,
    ) -> Result<PanelSufficiency, OracleError> {
        let store = AssayStore::load_from_vault(self.vault).map_err(OracleError::from)?;
        let key = AssayCacheKey::scoped(
            panel.version,
            domain.as_str(),
            self.vault.vault_id(),
            AnchorKind::Reward,
        );
        let panel_bits = bits(required_row(&store, &key, &AssaySubject::Panel)?, "panel")?;
        let outcome_entropy_bits = bits(
            required_row(&store, &key, &AssaySubject::OutcomeEntropy)?,
            "outcome entropy",
        )?;
        let slot_bits = panel
            .slots
            .iter()
            .map(|slot| {
                let row = required_row(&store, &key, &AssaySubject::Lens { slot: slot.slot_id })?;
                Ok((slot.slot_id, bits(row, "lens")?))
            })
            .collect::<Result<Vec<_>, OracleError>>()?;

        let attributions = per_sensor_attribution(&slot_bits, SOLE_CARRIER_BITS);
        Ok(panel_sufficiency_with_context(
            panel_bits,
            outcome_entropy_bits,
            &attributions,
            trust(&store, &key),
            DeficitRoutingContext {
                panel_id: format!("oracle:{domain}:panel:{}", panel.version),
                anchor: AnchorKind::Reward,
                computed_at_seq: clock.now(),
                observation_scope: None,
            },
        ))
    }
}

fn required_row<'a>(
    store: &'a AssayStore,
    key: &AssayCacheKey,
    subject: &AssaySubject,
) -> Result<&'a AssayRow, OracleError> {
    store.get(key, subject).ok_or_else(|| {
        CalyxError::assay_insufficient_samples(format!(
            "missing oracle sufficiency assay row for subject {subject:?}"
        ))
        .into()
    })
}

fn bits(row: &AssayRow, label: &str) -> Result<f32, OracleError> {
    let bits = row.estimate.bits;
    if bits.is_finite() && bits >= 0.0 {
        Ok(bits)
    } else {
        Err(CalyxError::aster_corrupt_shard(format!(
            "oracle sufficiency {label} bits must be finite and non-negative"
        ))
        .into())
    }
}

fn trust(store: &AssayStore, key: &AssayCacheKey) -> TrustTag {
    store
        .get(key, &AssaySubject::Panel)
        .map(|row| row.estimate.trust)
        .unwrap_or(TrustTag::Provisional)
}

fn validate_report(report: &PanelSufficiency) -> Result<(), OracleError> {
    if report.panel_bits.is_finite()
        && report.panel_bits >= 0.0
        && report.anchor_entropy_bits.is_finite()
        && report.anchor_entropy_bits >= 0.0
    {
        Ok(())
    } else {
        Err(
            CalyxError::assay_insufficient_samples("oracle sufficiency report has invalid bits")
                .into(),
        )
    }
}

fn lens_deficits(
    panel: &Panel,
    report: &PanelSufficiency,
) -> Result<Vec<(LensId, f32)>, OracleError> {
    if panel.slots.is_empty() {
        return Ok(Vec::new());
    }

    let mut per_slot = BTreeMap::<SlotId, f32>::new();
    for deficit in &report.deficits {
        if let Some(slot) = deficit.slot {
            per_slot.entry(slot).or_insert(deficit.deficit_bits);
        }
        for (slot, gap) in &deficit.per_slot_gaps {
            per_slot.insert(*slot, *gap);
        }
    }

    let mut by_lens = BTreeMap::<LensId, f32>::new();
    for slot in &panel.slots {
        if let Some(gap) = per_slot.get(&slot.slot_id).copied()
            && gap.is_finite()
            && gap > 0.0
        {
            *by_lens.entry(slot.lens_id).or_default() += gap;
        }
    }

    if by_lens.is_empty() {
        return Err(CalyxError::assay_insufficient_samples(
            "oracle insufficiency lacks per-sensor deficit attribution",
        )
        .into());
    }
    Ok(by_lens.into_iter().collect())
}

#[cfg(test)]
#[path = "honesty_gate_tests.rs"]
mod tests;
