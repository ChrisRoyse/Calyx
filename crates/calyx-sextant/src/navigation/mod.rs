//! Navigation modes over the constellation space (PRD 10 §4, §9).
//!
//! `neighbors`/`define`/`compare_lenses` are the per-lens primitives;
//! `agree`/`disagree` add cross-lens consensus/anomaly search,
//! `traverse` adds the asymmetric hop-attenuated walk, and
//! `skills`/`search_skill` add hierarchical skill navigation.

mod consensus;
mod hdbscan;
mod lens_nav;
mod skills;
mod traverse;

pub use consensus::{ConsensusHit, ConsensusMode, ConsensusReport, SlotCosine, agree, disagree};
pub use lens_nav::{LensComparison, compare_lenses, define, neighbors};
pub use skills::{SkillNode, SkillParams, SkillTree, search_skill, skills};
pub use traverse::{
    MAX_TRAVERSE_HOPS, TraverseDirection, TraversePath, TraverseStep, traverse, traverse_graph,
};
