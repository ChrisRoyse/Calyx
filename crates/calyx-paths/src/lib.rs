#![deny(warnings)]

//! Path and graph traversal over Calyx association networks.

pub mod attenuation;
mod error;
pub mod graph;
pub mod traversal;

pub use attenuation::{attenuate, deattenuate};
pub use error::{PathsError, Result};
pub use graph::{AssocGraph, AssocGraphBuilder, Edge, NodeEntry};
pub use traversal::{BidirectionalPath, bidirectional, reach, reach_scored};
