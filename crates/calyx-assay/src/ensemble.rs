mod compute;
mod model;

pub use compute::{CALYX_ASSAY_PANEL_TOO_SMALL, ensemble_card};
pub use model::{
    DEFAULT_GATE_PANEL_LENSES, DEFAULT_MAX_REDUNDANCY, DEFAULT_MIN_MARGINAL_BITS, DeficitProposal,
    ENSEMBLE_CARD_PID_METHOD, ENSEMBLE_CARD_SCHEMA_VERSION, EnsembleCard, EnsembleConfig,
    EnsembleDecision, EnsembleLensInput, EnsembleLensValue, EnsemblePairValue,
    MIN_ENSEMBLE_PANEL_LENSES, PidBits,
};

#[cfg(test)]
mod tests;
