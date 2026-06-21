//! Search, kernel-answer, and neighbors MCP tools for PH63 T04.

mod engine;
#[cfg(test)]
mod extension_tests;
mod extensions;
mod output;
#[cfg(test)]
mod tests;

use calyx_core::{AnchorKind, CalyxError, CxId, SlotId};
use calyx_sextant::{FusionStrategy, RrfProfile};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::protocol::ToolDef;
use crate::schema::{boolean_schema, integer_schema, object_schema, string_schema};
use crate::server::{McpServer, Tool, ToolError, ToolResult};

const DEFAULT_K: usize = 10;
const MAX_K: usize = 1000;

pub fn register(server: &mut McpServer) -> Result<(), CalyxError> {
    server.register(Box::new(SearchTool))?;
    server.register(Box::new(KernelAnswerTool))?;
    server.register(Box::new(NeighborsTool))?;
    extensions::register(server)?;
    Ok(())
}

struct SearchTool;
struct KernelAnswerTool;
struct NeighborsTool;

impl Tool for SearchTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.search",
            "search a Calyx vault",
            "the everyday multi-lens search (RRF default, provenance attached)",
            object_schema(&[
                ("vault", string_schema(), true),
                ("query", string_schema(), true),
                ("k", integer_range(1, MAX_K), false),
                (
                    "fusion",
                    enum_string(&[
                        "rrf",
                        "weighted_rrf",
                        "single_lens",
                        "kernel_first",
                        "pipeline",
                    ]),
                    false,
                ),
                ("guard", enum_string(&["off", "in_region"]), false),
                ("explain", boolean_schema(), false),
                ("fresh", boolean_schema(), false),
                ("filter", json!({ "type": "object" }), false),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: SearchArgs = decode("calyx.search", params)?;
        let request = SearchRequest::from_args(args)?;
        if request.guard == SearchGuard::InRegion {
            return Err(CalyxError::guard_provisional(
                "search guard is not calibrated for this vault",
            )
            .into());
        }
        let outcome = engine::search(&request)?;
        Ok(json!({ "hits": output::render_hits(&outcome.hits, request.explain, None) }))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

impl Tool for KernelAnswerTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.kernel_answer",
            "answer via the grounded kernel skeleton",
            "answer via the grounded kernel skeleton",
            object_schema(&[
                ("vault", string_schema(), true),
                ("query", string_schema(), true),
                ("anchor", string_schema(), false),
                ("explain", boolean_schema(), false),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: KernelAnswerArgs = decode("calyx.kernel_answer", params)?;
        validate_text(&args.query, "query")?;
        let anchor = args.anchor.as_deref().map(parse_anchor_kind).transpose()?;
        let search = SearchRequest {
            vault: args.vault,
            query: args.query,
            k: DEFAULT_K,
            fusion: SearchFusion::KernelFirst,
            guard: SearchGuard::Off,
            explain: args.explain.unwrap_or(false),
            filter: None,
        };
        let outcome = engine::search(&search)?;
        serde_json::to_value(engine::kernel_report(
            &outcome.docs,
            &outcome.hits,
            anchor.as_ref(),
        )?)
        .map_err(encode_error)
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

impl Tool for NeighborsTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.neighbors",
            "return per-lens neighbors of a stored constellation",
            "per-lens neighborhood of a known constellation",
            object_schema(&[
                ("vault", string_schema(), true),
                ("cx_id", string_schema(), true),
                ("slot", integer_schema(), false),
                ("k", integer_range(1, MAX_K), false),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: NeighborsArgs = decode("calyx.neighbors", params)?;
        let request = NeighborsRequest::from_args(args)?;
        Ok(json!({ "neighbors": engine::neighbors(&request)? }))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

#[derive(Deserialize)]
struct SearchArgs {
    vault: String,
    query: String,
    k: Option<usize>,
    fusion: Option<String>,
    guard: Option<String>,
    explain: Option<bool>,
    fresh: Option<bool>,
    filter: Option<Value>,
}

#[derive(Deserialize)]
struct KernelAnswerArgs {
    vault: String,
    query: String,
    anchor: Option<String>,
    explain: Option<bool>,
}

#[derive(Deserialize)]
struct NeighborsArgs {
    vault: String,
    cx_id: String,
    slot: Option<u16>,
    k: Option<usize>,
}

pub(super) struct SearchRequest {
    pub(super) vault: String,
    pub(super) query: String,
    pub(super) k: usize,
    pub(super) fusion: SearchFusion,
    pub(super) guard: SearchGuard,
    pub(super) explain: bool,
    pub(super) filter: Option<Value>,
}

pub(super) struct NeighborsRequest {
    pub(super) vault: String,
    pub(super) cx_id: CxId,
    pub(super) slot: Option<SlotId>,
    pub(super) k: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SearchFusion {
    Rrf,
    WeightedRrf,
    SingleLens,
    KernelFirst,
    Pipeline,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SearchGuard {
    Off,
    InRegion,
}

impl SearchRequest {
    fn from_args(args: SearchArgs) -> ToolResult<Self> {
        validate_text(&args.query, "query")?;
        let _fresh_requested = args.fresh.unwrap_or(false);
        if args.filter.as_ref().is_some_and(|value| !value.is_object()) {
            return Err(ToolError::invalid_params("filter must be a JSON object"));
        }
        Ok(Self {
            vault: args.vault,
            query: args.query,
            k: parse_k(args.k)?,
            fusion: SearchFusion::parse(args.fusion.as_deref())?,
            guard: SearchGuard::parse(args.guard.as_deref())?,
            explain: args.explain.unwrap_or(false),
            filter: args.filter,
        })
    }
}

impl NeighborsRequest {
    fn from_args(args: NeighborsArgs) -> ToolResult<Self> {
        Ok(Self {
            vault: args.vault,
            cx_id: args.cx_id.parse::<CxId>().map_err(|err| {
                ToolError::invalid_params(format!("parse cx_id {}: {err}", args.cx_id))
            })?,
            slot: args.slot.map(SlotId::new),
            k: parse_k(args.k)?,
        })
    }
}

impl SearchFusion {
    fn parse(value: Option<&str>) -> ToolResult<Self> {
        match value.unwrap_or("rrf") {
            "rrf" => Ok(Self::Rrf),
            "weighted_rrf" | "weighted-rrf" => Ok(Self::WeightedRrf),
            "single_lens" | "single-lens" => Ok(Self::SingleLens),
            "kernel_first" | "kernel-first" => Ok(Self::KernelFirst),
            "pipeline" => Ok(Self::Pipeline),
            other => Err(ToolError::invalid_params(format!("unknown fusion {other}"))),
        }
    }

    pub(super) fn to_strategy(self, slots: &[SlotId]) -> ToolResult<FusionStrategy> {
        match self {
            Self::Rrf => Ok(FusionStrategy::Rrf),
            Self::WeightedRrf => Ok(FusionStrategy::WeightedRrf {
                profile: RrfProfile::General,
            }),
            Self::SingleLens => slots
                .first()
                .copied()
                .map(|slot| FusionStrategy::SingleLens { slot })
                .ok_or_else(|| {
                    ToolError::invalid_params("single_lens search has no active lens slot")
                }),
            Self::KernelFirst => Ok(FusionStrategy::WeightedRrf {
                profile: RrfProfile::Kernel,
            }),
            Self::Pipeline => Ok(FusionStrategy::Pipeline),
        }
    }
}

impl SearchGuard {
    fn parse(value: Option<&str>) -> ToolResult<Self> {
        match value.unwrap_or("off") {
            "off" => Ok(Self::Off),
            "in_region" | "in-region" => Ok(Self::InRegion),
            other => Err(ToolError::invalid_params(format!("unknown guard {other}"))),
        }
    }
}

pub(super) fn parse_k(value: Option<usize>) -> ToolResult<usize> {
    let k = value.unwrap_or(DEFAULT_K);
    if (1..=MAX_K).contains(&k) {
        Ok(k)
    } else {
        Err(ToolError::invalid_params(format!(
            "k must be between 1 and {MAX_K}"
        )))
    }
}

pub(super) fn parse_anchor_kind(value: &str) -> ToolResult<AnchorKind> {
    Ok(match value {
        "test_pass" | "test-pass" => AnchorKind::TestPass,
        "thumbs_up" | "thumbs-up" | "thumbs_down" | "thumbs-down" => AnchorKind::Thumbs,
        "speaker_match" | "speaker-match" => AnchorKind::SpeakerMatch,
        "style_hold" | "style-hold" => AnchorKind::StyleHold,
        label if label.starts_with("label:") && label.len() > "label:".len() => {
            AnchorKind::Label(label["label:".len()..].to_string())
        }
        other => {
            return Err(ToolError::invalid_params(format!(
                "unknown anchor kind {other}"
            )));
        }
    })
}

pub(super) fn validate_text(value: &str, field: &str) -> ToolResult<()> {
    if value.is_empty() {
        return Err(ToolError::invalid_params(format!(
            "{field} must not be empty"
        )));
    }
    Ok(())
}

pub(super) fn decode<T: DeserializeOwned>(tool: &str, params: Value) -> ToolResult<T> {
    serde_json::from_value(params)
        .map_err(|err| ToolError::invalid_params(format!("{tool} invalid arguments: {err}")))
}

fn encode_error(error: serde_json::Error) -> ToolError {
    CalyxError::aster_corrupt_shard(format!("encode search result: {error}")).into()
}

pub(super) fn def(name: &str, description: &str, use_when: &str, input_schema: Value) -> ToolDef {
    ToolDef {
        name: name.to_string(),
        description: description.to_string(),
        use_when: use_when.to_string(),
        input_schema,
    }
}

pub(super) fn enum_string(values: &[&str]) -> Value {
    json!({ "type": "string", "enum": values })
}

pub(super) fn integer_range(min: usize, max: usize) -> Value {
    json!({ "type": "integer", "minimum": min, "maximum": max })
}
