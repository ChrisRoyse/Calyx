//! Provenance and ops MCP tools for PH63 T07.

mod core;
mod ids;
mod quarantine;
mod status;
#[cfg(test)]
mod tests;

use calyx_core::CalyxError;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::protocol::ToolDef;
use crate::schema::{integer_schema, object_schema, string_schema};
use crate::server::{McpServer, Tool, ToolError, ToolResult};
use calyx_aster::ledger_view::AsterLedgerCfStore;

pub fn register(server: &mut McpServer) -> Result<(), CalyxError> {
    server.register(Box::new(ProvenanceTool))?;
    server.register(Box::new(AnswerTraceTool))?;
    server.register(Box::new(VerifyChainTool))?;
    server.register(Box::new(ReproduceTool))?;
    server.register(Box::new(AnnealStatusTool))?;
    Ok(())
}

struct ProvenanceTool;
struct AnswerTraceTool;
struct VerifyChainTool;
struct ReproduceTool;
struct AnnealStatusTool;

impl Tool for ProvenanceTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.provenance",
            "full lineage of a constellation",
            "full lineage of a constellation",
            object_schema(&[
                ("vault", string_schema(), true),
                ("cx_id", string_schema(), true),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: ProvenanceArgs = decode("calyx.provenance", params)?;
        Ok(json!(core::lineage(&args.vault, &args.cx_id)?))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

impl Tool for AnswerTraceTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.answer_trace",
            "full lineage of a kernel answer or search result",
            "full lineage of a kernel answer or search result",
            object_schema(&[("answer_id", string_schema(), true)]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: AnswerTraceArgs = decode("calyx.answer_trace", params)?;
        Ok(json!(core::answer_trace(&args.answer_id)?))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

impl Tool for VerifyChainTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.verify_chain",
            "verify the ledger hash-chain",
            "tamper check: verify the Ledger hash-chain over a range",
            object_schema(&[
                ("vault", string_schema(), true),
                ("from_seq", integer_schema(), false),
                ("to_seq", integer_schema(), false),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: VerifyChainArgs = decode("calyx.verify_chain", params)?;
        Ok(json!(core::verify_chain_report(
            &args.vault,
            args.from_seq,
            args.to_seq,
        )?))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

impl Tool for ReproduceTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.reproduce",
            "replay a claim to verify bit-parity",
            "replay a claim to verify bit-parity",
            object_schema(&[
                ("vault", string_schema(), true),
                ("answer_id", string_schema(), true),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: ReproduceArgs = decode("calyx.reproduce", params)?;
        let report = core::reproduce(&args.vault, &args.answer_id)?;
        if report.bit_parity {
            Ok(json!(report))
        } else {
            Err(CalyxError::reproduce_drift_exceeded(format!(
                "original_hash={} reproduced_hash={}",
                report.original_hash, report.reproduced_hash
            ))
            .into())
        }
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

impl Tool for AnnealStatusTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.anneal.status",
            "inspect self-optimization state",
            "self-optimization state, tripwires, proposals",
            object_schema(&[("vault", string_schema(), true)]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: VaultArgs = decode("calyx.anneal.status", params)?;
        Ok(json!(status::anneal_status(&args.vault)?))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

#[derive(Deserialize)]
struct ProvenanceArgs {
    vault: String,
    cx_id: String,
}

#[derive(Deserialize)]
struct AnswerTraceArgs {
    answer_id: String,
}

#[derive(Deserialize)]
struct VerifyChainArgs {
    vault: String,
    from_seq: Option<u64>,
    to_seq: Option<u64>,
}

#[derive(Deserialize)]
struct ReproduceArgs {
    vault: String,
    answer_id: String,
}

#[derive(Deserialize)]
struct VaultArgs {
    vault: String,
}

fn decode<T: DeserializeOwned>(tool: &str, params: Value) -> ToolResult<T> {
    serde_json::from_value(params)
        .map_err(|err| ToolError::invalid_params(format!("{tool} invalid arguments: {err}")))
}

fn def(name: &str, description: &str, use_when: &str, input_schema: Value) -> ToolDef {
    ToolDef {
        name: name.to_string(),
        description: description.to_string(),
        use_when: use_when.to_string(),
        input_schema,
    }
}

fn open_ledger_view(path: &std::path::Path) -> ToolResult<AsterLedgerCfStore> {
    AsterLedgerCfStore::open(path).map_err(|error| {
        let missing_state = error.code == "CALYX_LEDGER_CORRUPT"
            && (error.message.contains("requires real Aster ledger state")
                || error.message.contains("not an Aster vault directory"));
        if missing_state {
            CalyxError::aster_corrupt_shard(error.message).into()
        } else {
            error.into()
        }
    })
}
