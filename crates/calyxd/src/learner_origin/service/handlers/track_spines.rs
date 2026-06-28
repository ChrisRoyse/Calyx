use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{AnchorKind, CxId, content_address};
use calyx_ledger::EntryKind;
use calyx_lodestar::{
    AssocStore, CollectionId, FilterExpr, HierarchicalKernelParams, KernelGraphParams,
    KernelParams, PropagatedLabel, PropagationError, RegionDescriptor, RegionId, RegionStore,
    Scope, ScopeCache, SparseGraph, TenantId, build_hierarchical_kernel, materialize_scope,
    propagate_labels_with_decay, scope_hash,
};
use serde_json::{Value, json};

use crate::learner_origin::model::{KIND_TRACK_SPINES, TrackSpinesRequest};
use crate::learner_origin::privacy::reject_private_material;

use super::super::storage::OriginCommit;
use super::super::{
    LearnerOriginService, OriginError, OriginResponse, STATUS_CREATED, STATUS_UNPROCESSABLE,
    base_metadata, ensure_nonempty, hex, insert_optional, now_millis, parse_body, sha256_array,
    sha256_hex, stable_id,
};
use super::shared::require_unit_interval;
use super::{TRACK_SPINES_EVIDENCE_KIND, TRACK_SPINES_PANEL_VERSION};

impl LearnerOriginService {
    pub(in crate::learner_origin::service) fn handle_track_spines(
        &self,
        body: &[u8],
    ) -> Result<OriginResponse, OriginError> {
        let value = parse_body(body)?;
        reject_private_material(&value)
            .map_err(|detail| OriginError::bad_request("CALYX_ORIGIN_PRIVATE_FIELD", detail))?;
        let request: TrackSpinesRequest = serde_json::from_value(value)
            .map_err(|error| OriginError::bad_request("CALYX_ORIGIN_JSON_INVALID", error))?;
        ensure_nonempty("learnerId", &request.learner_id)?;
        let domain = request
            .domain
            .as_deref()
            .unwrap_or("calyxweb-learner-kernels");
        ensure_nonempty("domain", domain)?;
        let body_hash = sha256_hex(body);
        let request_id = request.request_id.clone().unwrap_or_else(|| {
            stable_id(
                "track-spines",
                [request.learner_id.as_str(), domain, body_hash.as_str()],
            )
        });
        if let Some(existing) = self.find_by_idempotency(
            KIND_TRACK_SPINES,
            "request_id",
            &request_id,
            request.idempotency_key.as_deref(),
        )? {
            return self.duplicate_response(
                KIND_TRACK_SPINES,
                "requestId",
                &request_id,
                &body_hash,
                existing,
            );
        }

        let now = request.now_millis.unwrap_or_else(now_millis);
        let plan = TrackSpinesPlan::from_request(&request, domain, &request_id, now)?;
        let source_row =
            self.commit_track_spines_source(&request, &plan, &request_id, &body_hash)?;
        let output = plan.run()?;
        if output.provisional_positive_count == 0 {
            return Err(OriginError::new(
                STATUS_UNPROCESSABLE,
                "CALYX_ORIGIN_TRACK_SPINES_UNGROUNDED",
                "label propagation produced no positive provisional mastery labels",
            ));
        }

        let stored = self.commit_track_spines_result(
            &request,
            &plan,
            &output,
            &source_row.cx_id,
            &body_hash,
        )?;
        self.metrics.record_write(KIND_TRACK_SPINES, "accepted");
        Ok(OriginResponse::json(
            STATUS_CREATED,
            json!({
                "accepted": true,
                "duplicate": false,
                "requestId": request_id,
                "learnerId": request.learner_id,
                "domain": domain,
                "source": {
                    "cxId": source_row.cx_id,
                    "ledgerSeq": source_row.ledger_seq,
                    "ledgerHash": source_row.ledger_hash,
                    "nodeCount": plan.graph.node_count(),
                    "edgeCount": plan.graph.edge_count(),
                    "trackCount": plan.track_count(),
                    "masteryLabelCount": plan.kernel_labels.len()
                },
                "tracks": output.track_reports,
                "labelPropagation": {
                    "kernelLabelCount": plan.kernel_labels.len(),
                    "labelCount": output.label_count,
                    "provisionalPositiveCount": output.provisional_positive_count,
                    "maxHopDistance": output.max_hop_distance,
                    "decayLambda": plan.decay_lambda,
                    "labels": output.label_rows
                },
                "cxId": stored.cx_id,
                "ledgerSeq": stored.ledger_seq,
                "ledgerHash": stored.ledger_hash
            }),
        ))
    }

    fn commit_track_spines_source(
        &self,
        request: &TrackSpinesRequest,
        plan: &TrackSpinesPlan,
        request_id: &str,
        body_hash: &str,
    ) -> Result<super::super::storage::StoredRow, OriginError> {
        let mut metadata = base_metadata(TRACK_SPINES_EVIDENCE_KIND, body_hash);
        metadata.insert("request_id".to_string(), request_id.to_string());
        metadata.insert("learner_id".to_string(), request.learner_id.clone());
        metadata.insert("domain".to_string(), plan.domain.clone());
        metadata.insert(
            "graph_node_count".to_string(),
            plan.graph.node_count().to_string(),
        );
        metadata.insert(
            "graph_edge_count".to_string(),
            plan.graph.edge_count().to_string(),
        );
        metadata.insert("track_count".to_string(), plan.track_count().to_string());
        metadata.insert(
            "mastery_label_count".to_string(),
            plan.kernel_labels.len().to_string(),
        );
        insert_optional(
            &mut metadata,
            "idempotency_key",
            request.idempotency_key.as_deref(),
        );
        insert_optional(&mut metadata, "session_id", request.session_id.as_deref());
        insert_optional(
            &mut metadata,
            "privacy_class",
            request.privacy_class.as_deref(),
        );
        self.commit_origin_row(OriginCommit {
            kind: TRACK_SPINES_EVIDENCE_KIND,
            primary_id: request_id.to_string(),
            ledger_kind: EntryKind::Ingest,
            metadata,
            scalars: BTreeMap::from([
                (
                    "track_spines.graph_nodes".to_string(),
                    plan.graph.node_count() as f64,
                ),
                (
                    "track_spines.graph_edges".to_string(),
                    plan.graph.edge_count() as f64,
                ),
                ("track_spines.tracks".to_string(), plan.track_count() as f64),
            ]),
            slot_values: [
                7.0,
                plan.graph.node_count() as f32,
                plan.track_count() as f32,
                plan.kernel_labels.len() as f32,
            ],
            anchors: Vec::new(),
        })
    }

    fn commit_track_spines_result(
        &self,
        request: &TrackSpinesRequest,
        plan: &TrackSpinesPlan,
        output: &TrackSpinesOutput,
        source_cx_id: &str,
        body_hash: &str,
    ) -> Result<super::super::storage::StoredRow, OriginError> {
        let mut metadata = base_metadata(KIND_TRACK_SPINES, body_hash);
        metadata.insert("request_id".to_string(), output.request_id.clone());
        metadata.insert("learner_id".to_string(), request.learner_id.clone());
        metadata.insert("domain".to_string(), plan.domain.clone());
        metadata.insert("source_cx_id".to_string(), source_cx_id.to_string());
        metadata.insert("track_count".to_string(), plan.track_count().to_string());
        metadata.insert("label_count".to_string(), output.label_count.to_string());
        metadata.insert(
            "provisional_positive_count".to_string(),
            output.provisional_positive_count.to_string(),
        );
        metadata.insert(
            "max_hop_distance".to_string(),
            output.max_hop_distance.to_string(),
        );
        insert_optional(
            &mut metadata,
            "idempotency_key",
            request.idempotency_key.as_deref(),
        );
        insert_optional(&mut metadata, "session_id", request.session_id.as_deref());
        insert_optional(
            &mut metadata,
            "privacy_class",
            request.privacy_class.as_deref(),
        );
        self.commit_origin_row(OriginCommit {
            kind: KIND_TRACK_SPINES,
            primary_id: output.request_id.clone(),
            ledger_kind: EntryKind::Kernel,
            metadata,
            scalars: BTreeMap::from([
                (
                    "track_spines.track_count".to_string(),
                    plan.track_count() as f64,
                ),
                (
                    "track_spines.label_count".to_string(),
                    output.label_count as f64,
                ),
                (
                    "track_spines.provisional_positive_count".to_string(),
                    output.provisional_positive_count as f64,
                ),
            ]),
            slot_values: [
                8.0,
                plan.track_count() as f32,
                output.label_count as f32,
                output.provisional_positive_count as f32,
            ],
            anchors: Vec::new(),
        })
    }
}

struct TrackSpinesPlan {
    request_id: String,
    domain: String,
    graph: SparseGraph,
    node_to_concept: BTreeMap<CxId, String>,
    store: TrackRegionStore,
    kernel_labels: Vec<(CxId, f32)>,
    kernel_params: HierarchicalKernelParams,
    max_iter: usize,
    tol: f32,
    decay_lambda: f32,
}

impl TrackSpinesPlan {
    fn from_request(
        request: &TrackSpinesRequest,
        domain: &str,
        request_id: &str,
        now: u64,
    ) -> Result<Self, OriginError> {
        if request.nodes.is_empty() {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_EMPTY_TRACK_GRAPH",
                "track spine request must include at least one node",
            ));
        }
        if request.edges.is_empty() {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_EMPTY_TRACK_GRAPH",
                "track spine request must include at least one edge",
            ));
        }
        if request.tracks.is_empty() {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_EMPTY_TRACKS",
                "track spine request must include at least one track",
            ));
        }
        if request.mastery_labels.is_empty() {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_EMPTY_MASTERY_LABELS",
                "track spine request must include at least one probed mastery label",
            ));
        }

        let mut concept_to_node = BTreeMap::new();
        let mut node_to_concept = BTreeMap::new();
        let mut builder = SparseGraph::builder();
        for node in &request.nodes {
            ensure_nonempty("nodes.conceptId", &node.concept_id)?;
            let cx = concept_cx(domain, &node.concept_id);
            if concept_to_node
                .insert(node.concept_id.clone(), cx)
                .is_some()
            {
                return Err(OriginError::bad_request(
                    "CALYX_ORIGIN_DUPLICATE_CONCEPT",
                    format!("duplicate node conceptId `{}`", node.concept_id),
                ));
            }
            node_to_concept.insert(cx, node.concept_id.clone());
            builder
                .add_node(cx, node_weight(node.weight)?)
                .map_err(|error| lodestar_origin_error(error.into()))?;
        }
        for edge in &request.edges {
            ensure_nonempty("edges.fromConceptId", &edge.from_concept_id)?;
            ensure_nonempty("edges.toConceptId", &edge.to_concept_id)?;
            let from = node_for(&concept_to_node, &edge.from_concept_id)?;
            let to = node_for(&concept_to_node, &edge.to_concept_id)?;
            builder
                .add_edge(from, to, edge_weight(edge.weight)?)
                .map_err(|error| lodestar_origin_error(error.into()))?;
        }
        let graph = builder.build();
        if graph.is_empty() {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_EMPTY_TRACK_GRAPH",
                "track spine graph materialized to zero nodes",
            ));
        }

        let mut collections = BTreeMap::new();
        let mut regions_by_track = BTreeMap::new();
        let mut track_labels = BTreeMap::new();
        for track in &request.tracks {
            ensure_nonempty("tracks.trackId", &track.track_id)?;
            if track.regions.is_empty() {
                return Err(OriginError::bad_request(
                    "CALYX_ORIGIN_EMPTY_TRACK_REGIONS",
                    format!(
                        "track `{}` must include at least one region",
                        track.track_id
                    ),
                ));
            }
            let track_id = CollectionId(track.track_id.clone());
            if collections.contains_key(&track_id) {
                return Err(OriginError::bad_request(
                    "CALYX_ORIGIN_DUPLICATE_TRACK",
                    format!("duplicate trackId `{}`", track.track_id),
                ));
            }
            track_labels.insert(
                track_id.clone(),
                track
                    .label
                    .clone()
                    .unwrap_or_else(|| track.track_id.clone()),
            );
            let mut track_nodes = BTreeSet::new();
            let mut regions = Vec::new();
            for region in &track.regions {
                ensure_nonempty("tracks.regions.regionId", &region.region_id)?;
                ensure_nonempty(
                    "tracks.regions.centroidConceptId",
                    &region.centroid_concept_id,
                )?;
                if region.concept_ids.is_empty() {
                    return Err(OriginError::bad_request(
                        "CALYX_ORIGIN_EMPTY_TRACK_REGION",
                        format!(
                            "track `{}` region `{}` must include conceptIds",
                            track.track_id, region.region_id
                        ),
                    ));
                }
                let centroid = node_for(&concept_to_node, &region.centroid_concept_id)?;
                let mut members = BTreeSet::new();
                for concept_id in &region.concept_ids {
                    let cx = node_for(&concept_to_node, concept_id)?;
                    members.insert(cx);
                    track_nodes.insert(cx);
                }
                if !members.contains(&centroid) {
                    return Err(OriginError::bad_request(
                        "CALYX_ORIGIN_REGION_CENTROID_OUTSIDE_REGION",
                        format!(
                            "track `{}` region `{}` centroid must be present in conceptIds",
                            track.track_id, region.region_id
                        ),
                    ));
                }
                regions.push(RegionDescriptor {
                    id: RegionId(region.region_id.clone()),
                    centroid_cx: centroid,
                    members,
                });
            }
            collections.insert(track_id.clone(), track_nodes);
            regions_by_track.insert(track_id, regions);
        }

        let mut kernel_labels = Vec::new();
        for label in &request.mastery_labels {
            ensure_nonempty("masteryLabels.conceptId", &label.concept_id)?;
            let mastery = require_unit_interval("masteryLabels.mastery", label.mastery)?;
            kernel_labels.push((node_for(&concept_to_node, &label.concept_id)?, mastery));
        }
        kernel_labels.sort_by_key(|(node, _)| *node);
        kernel_labels.dedup_by_key(|(node, _)| *node);

        let max_iter = request.params.max_iter.unwrap_or(64);
        let tol = request.params.tol.unwrap_or(1.0e-6);
        let decay_lambda = request
            .params
            .decay_lambda
            .unwrap_or(calyx_lodestar::DEFAULT_PROPAGATION_DECAY_LAMBDA);
        let kernel_params = HierarchicalKernelParams {
            max_regions: request.params.max_regions.unwrap_or(16),
            drill_radius: request.params.drill_radius.unwrap_or(2),
            min_region_size: request.params.min_region_size.unwrap_or(1),
            anchor_kind: None,
            kernel_params: KernelParams {
                panel_version: TRACK_SPINES_PANEL_VERSION,
                anchor_kind: None,
                corpus_shard_hash: sha256_array(
                    format!("track-spines:{domain}:{request_id}").as_bytes(),
                ),
                built_at_millis: now,
                kernel_graph: KernelGraphParams {
                    target_fraction: 1.0,
                    max_groundedness_distance: 4,
                    ..KernelGraphParams::default()
                },
                ..KernelParams::default()
            },
        };
        Ok(Self {
            request_id: request_id.to_string(),
            domain: domain.to_string(),
            graph: graph.clone(),
            node_to_concept,
            store: TrackRegionStore {
                graph,
                collections,
                regions_by_track,
                track_labels,
            },
            kernel_labels,
            kernel_params,
            max_iter,
            tol,
            decay_lambda,
        })
    }

    fn run(&self) -> Result<TrackSpinesOutput, OriginError> {
        let mut cache = ScopeCache::new(self.track_count().max(1) * 4);
        let mut track_reports = Vec::new();
        for track_id in self.store.collections.keys() {
            let scope = Scope::Collection {
                id: track_id.clone(),
            };
            let scoped = materialize_scope(&scope, &self.store).map_err(lodestar_origin_error)?;
            if scoped.is_empty() {
                return Err(OriginError::new(
                    STATUS_UNPROCESSABLE,
                    "CALYX_ORIGIN_EMPTY_TRACK_SCOPE",
                    format!("track `{}` materialized to zero nodes", track_id.0),
                ));
            }
            let hierarchical = build_hierarchical_kernel(
                &self.store,
                scope.clone(),
                &self.kernel_params,
                &mut cache,
            )
            .map_err(lodestar_origin_error)?;
            let drilldowns = hierarchical
                .region_drilldowns
                .iter()
                .map(|(region_id, kernel)| {
                    json!({
                        "regionId": region_id.0,
                        "kernelSize": kernel.members.len(),
                        "members": self.concepts_for_nodes(&kernel.members),
                        "recallRatio": kernel.recall.ratio,
                        "unanchoredMemberCount": kernel.groundedness.unanchored_members.len()
                    })
                })
                .collect::<Vec<_>>();
            let all_members = hierarchical.all_members();
            let label = self
                .store
                .track_labels
                .get(track_id)
                .cloned()
                .unwrap_or_else(|| track_id.0.clone());
            track_reports.push(json!({
                "trackId": track_id.0,
                "label": label,
                "scopeHash": hex(&scope_hash(&scope)),
                "scopedNodeCount": scoped.node_count(),
                "regionKernelSize": hierarchical.region_kernel.members.len(),
                "drilldownCount": drilldowns.len(),
                "allMemberCount": all_members.len(),
                "allMembers": self.concepts_for_nodes(&all_members),
                "drilldowns": drilldowns
            }));
        }

        let labels = propagate_labels_with_decay(
            &self.graph,
            &self.kernel_labels,
            self.max_iter,
            self.tol,
            self.decay_lambda,
        )
        .map_err(propagation_origin_error)?;
        let provisional_positive_count = labels
            .iter()
            .filter(|label| label.provisional && label.confidence > 0.0)
            .count();
        let max_hop_distance = labels
            .iter()
            .filter_map(|label| (label.hop_distance != u32::MAX).then_some(label.hop_distance))
            .max()
            .unwrap_or(0);
        let label_rows = labels
            .iter()
            .map(|label| self.label_json(label))
            .collect::<Vec<_>>();
        Ok(TrackSpinesOutput {
            request_id: self.request_id.clone(),
            track_reports,
            label_count: labels.len(),
            provisional_positive_count,
            max_hop_distance,
            label_rows,
        })
    }

    fn track_count(&self) -> usize {
        self.store.collections.len()
    }

    fn concepts_for_nodes(&self, nodes: &[CxId]) -> Vec<String> {
        let mut concepts = nodes
            .iter()
            .map(|node| {
                self.node_to_concept
                    .get(node)
                    .cloned()
                    .unwrap_or_else(|| node.to_string())
            })
            .collect::<Vec<_>>();
        concepts.sort();
        concepts
    }

    fn label_json(&self, label: &PropagatedLabel) -> Value {
        json!({
            "conceptId": self
                .node_to_concept
                .get(&label.node_id)
                .cloned()
                .unwrap_or_else(|| label.node_id.to_string()),
            "nodeId": label.node_id.to_string(),
            "label": label.label,
            "confidence": label.confidence,
            "hopDistance": label.hop_distance,
            "provisional": label.provisional
        })
    }
}

struct TrackSpinesOutput {
    request_id: String,
    track_reports: Vec<Value>,
    label_count: usize,
    provisional_positive_count: usize,
    max_hop_distance: u32,
    label_rows: Vec<Value>,
}

#[derive(Clone)]
struct TrackRegionStore {
    graph: SparseGraph,
    collections: BTreeMap<CollectionId, BTreeSet<CxId>>,
    regions_by_track: BTreeMap<CollectionId, Vec<RegionDescriptor>>,
    track_labels: BTreeMap<CollectionId, String>,
}

impl AssocStore for TrackRegionStore {
    fn full_graph(&self) -> calyx_lodestar::Result<SparseGraph> {
        Ok(self.graph.clone())
    }

    fn collection_nodes(
        &self,
        id: &CollectionId,
    ) -> calyx_lodestar::Result<Option<BTreeSet<CxId>>> {
        Ok(self.collections.get(id).cloned())
    }

    fn domain_anchors(&self, _kind: &AnchorKind) -> calyx_lodestar::Result<Vec<CxId>> {
        Ok(Vec::new())
    }

    fn time_window_nodes(
        &self,
        _t0: calyx_core::Ts,
        _t1: calyx_core::Ts,
    ) -> calyx_lodestar::Result<Option<BTreeSet<CxId>>> {
        Ok(None)
    }

    fn tenant_nodes(&self, _id: &TenantId) -> calyx_lodestar::Result<Option<BTreeSet<CxId>>> {
        Ok(None)
    }

    fn filter_nodes(&self, _expr: &FilterExpr) -> calyx_lodestar::Result<BTreeSet<CxId>> {
        Ok(BTreeSet::new())
    }
}

impl RegionStore for TrackRegionStore {
    fn regions_for_scope(&self, scope: &Scope) -> calyx_lodestar::Result<Vec<RegionDescriptor>> {
        let Scope::Collection { id } = scope else {
            return Ok(Vec::new());
        };
        Ok(self.regions_by_track.get(id).cloned().unwrap_or_default())
    }
}

fn concept_cx(domain: &str, concept_id: &str) -> CxId {
    CxId::from_bytes(content_address([
        b"calyx-learner-track-concept-v1".as_slice(),
        domain.as_bytes(),
        concept_id.as_bytes(),
    ]))
}

fn node_for(nodes: &BTreeMap<String, CxId>, concept_id: &str) -> Result<CxId, OriginError> {
    nodes.get(concept_id).copied().ok_or_else(|| {
        OriginError::bad_request(
            "CALYX_ORIGIN_UNKNOWN_CONCEPT",
            format!("conceptId `{concept_id}` is not present in nodes"),
        )
    })
}

fn node_weight(weight: Option<f32>) -> Result<f32, OriginError> {
    let value = weight.unwrap_or(1.0);
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(OriginError::bad_request(
            "CALYX_ORIGIN_INVALID_NUMBER",
            "node weight must be finite and positive",
        ))
    }
}

fn edge_weight(weight: Option<f32>) -> Result<f32, OriginError> {
    let value = weight.unwrap_or(1.0);
    if value.is_finite() && value > 0.0 && value <= 1.0 {
        Ok(value)
    } else {
        Err(OriginError::bad_request(
            "CALYX_ORIGIN_INVALID_NUMBER",
            "edge weight must be finite and within (0, 1]",
        ))
    }
}

fn lodestar_origin_error(error: calyx_lodestar::LodestarError) -> OriginError {
    OriginError::new(
        STATUS_UNPROCESSABLE,
        "CALYX_ORIGIN_TRACK_SPINES_REJECTED",
        format!("{}: {}", error.code(), error),
    )
}

fn propagation_origin_error(error: PropagationError) -> OriginError {
    OriginError::new(
        STATUS_UNPROCESSABLE,
        "CALYX_ORIGIN_LABEL_PROPAGATION_REJECTED",
        format!("{}: {}", error.code(), error),
    )
}
