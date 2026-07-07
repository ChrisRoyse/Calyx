use calyx_core::Seq;
use calyx_loom::agreement_graph::{AgreementEdge, XtermRow};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DenseGroupReport {
    pub dim: u32,
    pub slots: Vec<u16>,
    pub xterm_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShapeAwareConstellationReport {
    pub cx_id: String,
    pub panel_version: u32,
    pub slot_count: usize,
    pub dense_groups: Vec<DenseGroupReport>,
    pub sparse_agreement_count: usize,
    pub unsupported_pair_count: usize,
    pub xterm_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnsupportedShapePair {
    pub cx_id: String,
    pub slot_a: u16,
    pub slot_b: u16,
    pub shape_a: String,
    pub shape_b: String,
    pub reason_code: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShapeAwareLoomWeaveReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub domain: String,
    pub panel_version: u32,
    pub cache_capacity: usize,
    pub source_cx_ids: Vec<String>,
    pub constellation_count: usize,
    pub xterm_count: usize,
    pub unsupported_pair_count: usize,
    pub persisted_seq: Seq,
    pub constellations: Vec<ShapeAwareConstellationReport>,
    pub unsupported_pairs: Vec<UnsupportedShapePair>,
    pub xterm_rows: Vec<XtermRow>,
    pub xterm_order: Vec<String>,
    pub agreement_graph: Vec<AgreementEdge>,
    pub agreement_graph_order: Vec<String>,
    pub graph_source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShapeAwareLoomWeaveRun {
    pub report_path: PathBuf,
    pub report: ShapeAwareLoomWeaveReport,
    pub persisted_seq: Seq,
}
