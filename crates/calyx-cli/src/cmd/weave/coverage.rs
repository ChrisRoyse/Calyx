use std::collections::BTreeMap;

use calyx_core::{AnchorKind, Constellation, CxId, SlotId, Ts};
use calyx_lodestar::LodestarError;
use serde::Serialize;

use super::scratch::{OrdinalMap, WeaveVectorScratch};

const EXAMPLE_MISSING_LIMIT: usize = 5;
const COVERED_SCAN_BATCH_SIZE: usize = 64;

/// The only Base-row fields consumed after coverage selection. Keeping full
/// [`Constellation`] values retained payload refs, scalar maps, provenance,
/// flags, and placeholder slot values for the entire 109k-node weave. Project
/// at the selection boundary so post-scan memory is proportional to the graph
/// metadata contract, not the ingest record schema (#1999).
pub(super) struct WeaveNodeMeta {
    pub cx_id: CxId,
    pub created_at: Ts,
    pub anchor_kinds: Vec<AnchorKind>,
    pub metadata: BTreeMap<String, String>,
}

/// Minimal base-row state required only while coverage resolves physical slot
/// rows. Full [`Constellation`] values include retained input refs, scalar
/// maps, flags, and placeholder vectors that coverage never consumes. Project
/// immediately after decoding each base row so the full-corpus scan never
/// retains that payload (#1999).
pub(super) struct WeaveScanNode {
    pub meta: WeaveNodeMeta,
    pub provenance_seq: u64,
    pub listed_slots: Box<[SlotId]>,
}

impl From<Constellation> for WeaveScanNode {
    fn from(cx: Constellation) -> Self {
        Self {
            provenance_seq: cx.provenance.seq,
            listed_slots: cx.slots.into_keys().collect::<Vec<_>>().into_boxed_slice(),
            meta: WeaveNodeMeta {
                cx_id: cx.cx_id,
                created_at: cx.created_at,
                anchor_kinds: cx.anchors.into_iter().map(|anchor| anchor.kind).collect(),
                metadata: cx.metadata,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) enum CandidateSelectionMode {
    Covered,
    BasePrefix,
}

impl CandidateSelectionMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Covered => "covered",
            Self::BasePrefix => "base-prefix",
        }
    }
}

/// Exact per-slot typed coverage accounting. The counters are additive by
/// construction: `measured + absent + tombstoned + missing == candidate_rows`.
/// The four shape counters partition `measured_rows`. `missing_rows` counts only
/// candidates whose base row does
/// not list the slot — a base-listed slot with no physical row fails the scan
/// closed instead of being counted here (issue #1096).
#[derive(Clone, Debug, Serialize)]
pub(super) struct DenseSlotCoverage {
    pub slot_id: u16,
    pub candidate_rows: usize,
    pub dense_rows: usize,
    pub sparse_rows: usize,
    pub multi_rows: usize,
    pub segmented_multi_rows: usize,
    pub measured_rows: usize,
    pub missing_rows: usize,
    pub absent_rows: usize,
    pub tombstoned_rows: usize,
    pub example_missing_cx_ids: Vec<String>,
    pub read_stats: crate::provenance_read::ProvenanceReadStats,
}

/// Result of the coverage scan. The decoded corpus is **not** held here: every
/// measured vector lives in the out-of-core [`WeaveVectorScratch`] store keyed by
/// scan ordinal (index into `scanned_candidates`); `present_slot_counts[o]` is
/// how many contracted slots the candidate at ordinal `o` has a measured vector
/// in, which drives the complete-panel intersection without any resident vector
/// payload (#1991).
pub(super) struct DenseSlotCoverageScan {
    pub constellations_in_vault: usize,
    pub scanned_candidates: Vec<WeaveScanNode>,
    pub scanned_slots: Vec<SlotId>,
    pub present_slot_counts: Vec<u32>,
    pub scratch: WeaveVectorScratch,
    pub coverage: Vec<DenseSlotCoverage>,
    pub base_page_index_live_entries: usize,
    pub candidate_scan_rows: usize,
    pub candidate_scan_complete: bool,
}

/// The selected complete-panel candidate set, plus the out-of-core store and the
/// ordinal↔cx_id map the passes read vectors through. `candidate_ordinals[i]` is
/// the scan ordinal of `candidates[i]` in the scratch store.
pub(super) struct DenseSlotPreflight {
    pub constellations_in_vault: usize,
    pub candidates: Vec<WeaveNodeMeta>,
    pub candidate_ordinals: Vec<u64>,
    pub ordinal_map: OrdinalMap,
    pub scratch: WeaveVectorScratch,
    pub coverage: Vec<DenseSlotCoverage>,
    pub base_page_index_live_entries: usize,
    pub candidate_scan_rows: usize,
    pub candidate_scan_complete: bool,
    pub selected_candidate_rows: usize,
    pub selected_candidate_cx_ids: Vec<String>,
}

mod scan;
pub(super) use scan::scan_dense_slot_coverage;

/// The complete-panel intersection result — which scanned ordinals survive plus
/// the ordinal↔cx_id map — decoupled from the out-of-core store so it is
/// unit-testable without a filesystem.
pub(super) struct CompletePanelSelection {
    pub candidates: Vec<WeaveNodeMeta>,
    pub candidate_ordinals: Vec<u64>,
    pub ordinal_map: OrdinalMap,
    pub selected_candidate_cx_ids: Vec<String>,
}

/// Select only complete constellations: a selected node must have a measured
/// typed vector in every contracted panel slot (an intersection, never the
/// coverage set of one preferred lens). Pure over the scan's per-ordinal
/// present-slot counts — no vector payload, no filesystem — so the selection
/// contract is testable on any platform. `coverage` is only threaded through for
/// the fail-closed empty-panel diagnostic.
pub(super) fn select_complete_panel<T>(
    scanned_candidates: Vec<T>,
    present_slot_counts: &[u32],
    scanned_slots: &[SlotId],
    slots: &[SlotId],
    limit: usize,
    coverage: &[DenseSlotCoverage],
) -> Result<CompletePanelSelection, String>
where
    T: Into<WeaveScanNode>,
{
    if slots.len() < 2 {
        return Err(
            "CALYX_WEAVE_LOOM_PANEL_TOO_SMALL: need at least two typed content slots".to_string(),
        );
    }
    if scanned_slots != slots {
        return Err(format!(
            "CALYX_WEAVE_LOOM_PANEL_NOT_SCANNED: scanned slots {:?} differ from contracted slots {:?}",
            scanned_slots
                .iter()
                .map(|slot| slot.get())
                .collect::<Vec<_>>(),
            slots.iter().map(|slot| slot.get()).collect::<Vec<_>>()
        ));
    }
    let target = if limit == 0 { usize::MAX } else { limit };
    let full = slots.len() as u32;
    let mut candidates = Vec::new();
    let mut candidate_ordinals = Vec::new();
    let mut ordinal_map = OrdinalMap::default();
    let mut selected_candidate_cx_ids = Vec::new();
    for (ordinal, scan_node) in scanned_candidates.into_iter().map(Into::into).enumerate() {
        if candidates.len() >= target {
            break;
        }
        if present_slot_counts.get(ordinal).copied().unwrap_or(0) != full {
            continue;
        }
        ordinal_map.insert(scan_node.meta.cx_id, ordinal as u64);
        selected_candidate_cx_ids.push(scan_node.meta.cx_id.to_string());
        candidate_ordinals.push(ordinal as u64);
        candidates.push(scan_node.meta);
    }
    if candidates.len() < 2 {
        return Err(format!(
            "CALYX_WEAVE_LOOM_COMPLETE_PANEL_EMPTY: only {} constellations have all {} contracted typed slots; coverage={}",
            candidates.len(),
            slots.len(),
            coverage_summary(coverage)
        ));
    }
    Ok(CompletePanelSelection {
        candidates,
        candidate_ordinals,
        ordinal_map,
        selected_candidate_cx_ids,
    })
}

/// Materialize the panel preflight: run the complete-panel intersection and
/// attach the out-of-core store the passes read vectors through. The decoded
/// vectors stay out-of-core in `scan.scratch`.
pub(super) fn materialize_panel_preflight(
    scan: DenseSlotCoverageScan,
    slots: &[SlotId],
    limit: usize,
) -> Result<DenseSlotPreflight, String> {
    let selection = select_complete_panel(
        scan.scanned_candidates,
        &scan.present_slot_counts,
        &scan.scanned_slots,
        slots,
        limit,
        &scan.coverage,
    )?;
    Ok(DenseSlotPreflight {
        constellations_in_vault: scan.constellations_in_vault,
        selected_candidate_rows: selection.candidates.len(),
        candidate_scan_rows: scan.candidate_scan_rows,
        candidate_scan_complete: scan.candidate_scan_complete,
        candidates: selection.candidates,
        candidate_ordinals: selection.candidate_ordinals,
        ordinal_map: selection.ordinal_map,
        scratch: scan.scratch,
        coverage: scan.coverage,
        base_page_index_live_entries: scan.base_page_index_live_entries,
        selected_candidate_cx_ids: selection.selected_candidate_cx_ids,
    })
}

pub(super) fn coverage_summary(coverage: &[DenseSlotCoverage]) -> String {
    coverage
        .iter()
        .map(|row| {
            format!(
                "slot {} measured={}/{} dense={} sparse={} multi={} segmented_multi={} missing={} absent={} tombstoned={} examples={:?}",
                row.slot_id,
                row.measured_rows,
                row.candidate_rows,
                row.dense_rows,
                row.sparse_rows,
                row.multi_rows,
                row.segmented_multi_rows,
                row.missing_rows,
                row.absent_rows,
                row.tombstoned_rows,
                row.example_missing_cx_ids
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub(super) fn invalid_params(detail: impl Into<String>) -> crate::error::CliError {
    LodestarError::KernelInvalidParams {
        detail: detail.into(),
    }
    .into()
}
