use std::collections::BTreeMap;

use calyx_core::SlotId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RrfProfile {
    Causal,
    Code,
    Entity,
    Temporal,
    Speaker,
    Style,
    Civic,
    Media,
    Bridge,
    Kernel,
    Semantic,
    Lexical,
    Multimodal,
    General,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WeightedProfile {
    pub profile: RrfProfile,
    pub weights: BTreeMap<SlotId, f32>,
    pub lexical_excludes_dense: bool,
}

pub fn weighted_profiles() -> Vec<WeightedProfile> {
    use RrfProfile::*;
    [
        (Causal, &[4, 8, 18][..], false),
        (Code, &[8, 9, 10, 11, 16][..], false),
        (Entity, &[3, 8, 18][..], false),
        (Temporal, &[20, 21, 22][..], false),
        (Speaker, &[5, 8][..], false),
        (Style, &[6, 8][..], false),
        (Civic, &[1, 2, 3, 8][..], false),
        (Media, &[8, 9, 10][..], false),
        (Bridge, &[8, 14, 18][..], false),
        (Kernel, &[7, 8, 15][..], false),
        (Semantic, &[8][..], false),
        (Lexical, &[1][..], true),
        (Multimodal, &[8, 9, 10, 11][..], false),
        (General, &[1, 8, 18, 20][..], false),
    ]
    .into_iter()
    .map(|(profile, slots, lexical_excludes_dense)| WeightedProfile {
        profile,
        weights: slots
            .iter()
            .enumerate()
            .map(|(idx, slot)| (SlotId::new(*slot), 1.0 / (idx as f32 + 1.0)))
            .collect(),
        lexical_excludes_dense,
    })
    .collect()
}

pub fn lookup(profile: RrfProfile) -> Option<WeightedProfile> {
    weighted_profiles()
        .into_iter()
        .find(|candidate| candidate.profile == profile)
}
