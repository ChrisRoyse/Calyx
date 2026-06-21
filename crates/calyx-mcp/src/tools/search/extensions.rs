mod guard_generate;
mod render;
mod runtime;
mod xterms;

use calyx_core::{CalyxError, CxId, SlotId};
use calyx_sextant::{MAX_TRAVERSE_HOPS, SearchEngine, TraverseDirection, agree, disagree};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::schema::{integer_schema, object_schema, string_schema};
use crate::server::{McpServer, Tool, ToolError, ToolResult};

use super::output;
use super::{decode, def, enum_string, integer_range, validate_text};
use runtime::{
    NavRuntime, ensure_doc_exists, load_runtime, parse_cx_id, query_vector_for_skill, score01,
    skill_tree,
};

const CONSENSUS_K: usize = 5;
const SEARCH_SKILL_K: usize = 10;

pub(super) fn register(server: &mut McpServer) -> Result<(), CalyxError> {
    server.register(Box::new(AgreeTool))?;
    server.register(Box::new(DisagreeTool))?;
    server.register(Box::new(DefineTool))?;
    server.register(Box::new(GuardGenerateTool))?;
    server.register(Box::new(TraverseTool))?;
    server.register(Box::new(SkillsTool))?;
    server.register(Box::new(SearchSkillTool))?;
    Ok(())
}

struct AgreeTool;
struct DisagreeTool;
struct DefineTool;
struct GuardGenerateTool;
struct TraverseTool;
struct SkillsTool;
struct SearchSkillTool;

impl Tool for AgreeTool {
    fn def(&self) -> crate::protocol::ToolDef {
        def(
            "calyx.agree",
            "find constellations consistent with a stored constellation",
            "find constellations consistent with this one on a given lens",
            object_schema(&[
                ("vault", string_schema(), true),
                ("cx_id", string_schema(), true),
                ("slot", integer_schema(), false),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        consensus_call(decode("calyx.agree", params)?, ConsensusPolarity::Agree)
    }

    fn requires_authn(&self) -> bool {
        true
    }
}

impl Tool for DisagreeTool {
    fn def(&self) -> crate::protocol::ToolDef {
        def(
            "calyx.disagree",
            "find constellations anomalous relative to a stored constellation",
            "find constellations that are anomalous relative to this one",
            object_schema(&[
                ("vault", string_schema(), true),
                ("cx_id", string_schema(), true),
                ("slot", integer_schema(), false),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        consensus_call(
            decode("calyx.disagree", params)?,
            ConsensusPolarity::Disagree,
        )
    }

    fn requires_authn(&self) -> bool {
        true
    }
}

impl Tool for DefineTool {
    fn def(&self) -> crate::protocol::ToolDef {
        def(
            "calyx.define",
            "return the cross-lens definition for a lens coordinate",
            "get a term's grounded definition across the other lenses",
            object_schema(&[
                ("vault", string_schema(), true),
                ("lens", integer_schema(), true),
                ("index", integer_schema(), true),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: DefineArgs = decode("calyx.define", params)?;
        let runtime = load_runtime(&args.vault)?;
        let lens = SlotId::new(args.lens);
        let Some(cx_id) = runtime.docs.keys().copied().nth(args.index) else {
            return Ok(json!({ "definition": render::empty_definition(args.lens, args.index) }));
        };
        let definition = calyx_sextant::define(&runtime.engine, cx_id, lens, CONSENSUS_K)
            .map(render::definition)
            .unwrap_or_else(|_| render::empty_definition(args.lens, args.index));
        Ok(json!({ "definition": definition }))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

impl Tool for GuardGenerateTool {
    fn def(&self) -> crate::protocol::ToolDef {
        def(
            "calyx.guard_generate",
            "identity-locked generation gate",
            "accept generated text only if it stays inside calibrated Gtau slots",
            object_schema(&[
                ("vault", string_schema(), true),
                ("candidate_text", string_schema(), true),
                ("identity_cx", string_schema(), false),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: GuardGenerateArgs = decode("calyx.guard_generate", params)?;
        validate_text(&args.candidate_text, "candidate_text")?;
        let runtime = load_runtime(&args.vault)?;
        guard_generate::run(&runtime, &args.candidate_text, args.identity_cx.as_deref())
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

impl Tool for TraverseTool {
    fn def(&self) -> crate::protocol::ToolDef {
        def(
            "calyx.traverse",
            "walk the vault association graph from a constellation",
            "causal/asymmetric walk from a constellation",
            object_schema(&[
                ("vault", string_schema(), true),
                ("cx_id", string_schema(), true),
                (
                    "direction",
                    enum_string(&["forward", "backward", "both"]),
                    true,
                ),
                ("hops", integer_range(1, MAX_TRAVERSE_HOPS as usize), true),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: TraverseArgs = decode("calyx.traverse", params)?;
        if !(1..=MAX_TRAVERSE_HOPS).contains(&args.hops) {
            return Err(ToolError::invalid_params(format!(
                "hops must be between 1 and {MAX_TRAVERSE_HOPS}"
            )));
        }
        let cx_id = parse_cx_id(&args.cx_id)?;
        let runtime = load_runtime(&args.vault)?;
        ensure_doc_exists(&runtime.docs, cx_id)?;
        let direction = parse_direction(&args.direction)?;
        let path = calyx_sextant::traverse(&runtime.engine, cx_id, direction, args.hops)?;
        Ok(json!({
            "path": path.steps.into_iter().map(|step| json!({
                "cx_id": step.cx_id.to_string(),
                "hop": step.hop,
                "direction": render::direction_key(step.direction),
                "score": score01(step.score),
                "via": step.via.to_string(),
            })).collect::<Vec<_>>()
        }))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

impl Tool for SkillsTool {
    fn def(&self) -> crate::protocol::ToolDef {
        def(
            "calyx.skills",
            "return the hierarchical skill tree for a vault",
            "hierarchical-skill navigation",
            object_schema(&[("vault", string_schema(), true)]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: SkillsArgs = decode("calyx.skills", params)?;
        let runtime = load_runtime(&args.vault)?;
        Ok(json!({ "skill_tree": skill_tree(&runtime.engine)? }))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

impl Tool for SearchSkillTool {
    fn def(&self) -> crate::protocol::ToolDef {
        def(
            "calyx.search_skill",
            "search inside a named skill scope",
            "search within a specific skill scope",
            object_schema(&[
                ("vault", string_schema(), true),
                ("skill", string_schema(), true),
                ("query", string_schema(), true),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: SearchSkillArgs = decode("calyx.search_skill", params)?;
        validate_text(&args.skill, "skill")?;
        validate_text(&args.query, "query")?;
        let runtime = load_runtime(&args.vault)?;
        let tree = skill_tree(&runtime.engine)?;
        if !tree.nodes.contains_key(&args.skill) {
            return Ok(json!({ "hits": [] }));
        }
        let Some((slot, vector)) = query_vector_for_skill(&runtime, &args.query)? else {
            return Ok(json!({ "hits": [] }));
        };
        let mut query = calyx_sextant::Query::new(args.query)
            .with_vector(vector)
            .with_slots(vec![slot])
            .require_stored_provenance(true);
        query.k = SEARCH_SKILL_K;
        query.freshness = calyx_sextant::FreshnessRequirement::StaleOk { seq_lag: u64::MAX };
        let hits = calyx_sextant::search_skill(&runtime.engine, &tree, &args.skill, &query)?;
        Ok(json!({ "hits": output::render_hits(&hits, false, None) }))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

#[derive(Deserialize)]
struct ConsensusArgs {
    vault: String,
    cx_id: String,
    slot: Option<u16>,
}

#[derive(Deserialize)]
struct DefineArgs {
    vault: String,
    lens: u16,
    index: usize,
}

#[derive(Deserialize)]
struct GuardGenerateArgs {
    vault: String,
    candidate_text: String,
    identity_cx: Option<String>,
}

#[derive(Deserialize)]
struct TraverseArgs {
    vault: String,
    cx_id: String,
    direction: String,
    hops: u32,
}

#[derive(Deserialize)]
struct SkillsArgs {
    vault: String,
}

#[derive(Deserialize)]
struct SearchSkillArgs {
    vault: String,
    skill: String,
    query: String,
}

#[derive(Clone, Copy)]
enum ConsensusPolarity {
    Agree,
    Disagree,
}

fn consensus_call(args: ConsensusArgs, polarity: ConsensusPolarity) -> ToolResult<Value> {
    let cx_id = parse_cx_id(&args.cx_id)?;
    let runtime = load_runtime(&args.vault)?;
    ensure_doc_exists(&runtime.docs, cx_id)?;
    let rows = if let Some(slot) = args.slot {
        slot_consensus(&runtime, cx_id, SlotId::new(slot), polarity)?
    } else {
        xterms::materialize_agreement_xterms(&runtime.vault, &runtime.docs)?;
        match cross_lens_consensus(&runtime.engine, cx_id, polarity) {
            Ok(rows) => rows,
            Err(ToolError::Calyx(error))
                if error.code == "CALYX_SEXTANT_CONSENSUS_INSUFFICIENT_LENSES" =>
            {
                let slot = runtime
                    .engine
                    .indexes
                    .slots()
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        CalyxError::stale_derived("agree/disagree has no active indexable slot")
                    })?;
                slot_consensus(&runtime, cx_id, slot, polarity)?
            }
            Err(error) => return Err(error),
        }
    };
    Ok(json!({ "constellations": rows }))
}

fn slot_consensus(
    runtime: &NavRuntime,
    cx_id: CxId,
    slot: SlotId,
    polarity: ConsensusPolarity,
) -> ToolResult<Vec<Value>> {
    let vector = runtime
        .docs
        .get(&cx_id)
        .and_then(|cx| cx.slots.get(&slot))
        .ok_or_else(|| CalyxError::stale_derived(format!("cx_id {cx_id} lacks slot {slot}")))?;
    let mut hits =
        runtime
            .engine
            .indexes
            .search(slot, vector, runtime.docs.len().max(CONSENSUS_K), None)?;
    hits.retain(|hit| hit.cx_id != cx_id);
    if matches!(polarity, ConsensusPolarity::Disagree) {
        hits.sort_by(|a, b| {
            a.score
                .total_cmp(&b.score)
                .then_with(|| a.cx_id.cmp(&b.cx_id))
        });
    }
    Ok(hits
        .into_iter()
        .take(CONSENSUS_K)
        .map(|hit| {
            json!({
                "cx_id": hit.cx_id.to_string(),
                "score": score01(hit.score),
                "slot": slot.get(),
            })
        })
        .collect())
}

fn cross_lens_consensus(
    search: &SearchEngine,
    cx_id: CxId,
    polarity: ConsensusPolarity,
) -> ToolResult<Vec<Value>> {
    let report = match polarity {
        ConsensusPolarity::Agree => agree(search, cx_id, CONSENSUS_K, None)?,
        ConsensusPolarity::Disagree => disagree(search, cx_id, CONSENSUS_K, None)?,
    };
    Ok(report
        .hits
        .into_iter()
        .map(|hit| {
            json!({
                "cx_id": hit.cx_id.to_string(),
                "score": score01(hit.score),
                "slot": hit.per_slot.first().map(|slot| slot.slot.get()).unwrap_or(0),
            })
        })
        .collect())
}

fn parse_direction(value: &str) -> ToolResult<TraverseDirection> {
    match value {
        "forward" => Ok(TraverseDirection::Forward),
        "backward" => Ok(TraverseDirection::Backward),
        "both" => Ok(TraverseDirection::Both),
        other => Err(ToolError::invalid_params(format!(
            "unknown direction {other}"
        ))),
    }
}
