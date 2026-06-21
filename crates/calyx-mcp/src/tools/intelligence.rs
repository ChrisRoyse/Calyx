//! Intelligence extraction MCP tools for PH63 T06.

mod core;
mod guard;
mod metrics;
mod model;
mod propose;
mod propose_backfill;
mod propose_live;
mod propose_profile;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use calyx_assay::PanelResourceBudget;
use calyx_aster::cf::ColumnFamily;
use calyx_core::CalyxError;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::protocol::ToolDef;
use crate::schema::{boolean_schema, number_schema, object_schema, string_schema};
use crate::server::{McpServer, Tool, ToolError, ToolResult};

pub fn register(server: &mut McpServer) -> Result<(), CalyxError> {
    server.register(Box::new(AbundanceTool))?;
    server.register(Box::new(BitsTool))?;
    server.register(Box::new(KernelTool))?;
    server.register(Box::new(GuardCalibrateTool))?;
    server.register(Box::new(GuardCheckTool))?;
    server.register(Box::new(ProposeLensTool))?;
    Ok(())
}

struct AbundanceTool;
struct BitsTool;
struct KernelTool;
struct GuardCalibrateTool;
struct GuardCheckTool;
struct ProposeLensTool;

impl Tool for AbundanceTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.abundance",
            "DDA abundance report",
            "DDA report: N, C(N,2), materialized, n_eff, DPI ceiling",
            object_schema(&[("vault", string_schema(), true)]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: VaultArgs = decode("calyx.abundance", params)?;
        let ctx = core::load_context(&args.vault)?;
        let docs = core::load_docs(&ctx.vault)?;
        let slots = core::active_slot_ids(&ctx.state.panel);
        Ok(json!(metrics::abundance(&docs, &slots)))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

impl Tool for BitsTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.bits",
            "per-lens signal and panel sufficiency",
            "per-lens signal + panel sufficiency + deficit attribution",
            object_schema(&[
                ("vault", string_schema(), true),
                ("anchor", string_schema(), true),
                ("explain", boolean_schema(), false),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: BitsArgs = decode("calyx.bits", params)?;
        let ctx = core::load_context(&args.vault)?;
        let docs = core::load_docs(&ctx.vault)?;
        let anchor = core::parse_anchor(&args.anchor)?;
        let label = core::anchor_label(&anchor);
        let key = model::assay_key(&label);
        let report = metrics::bits(
            &ctx.state.panel,
            &docs,
            &anchor,
            &label,
            args.explain.unwrap_or(false),
            &key,
        )?;
        core::write_json_row(
            &ctx.vault,
            calyx_aster::cf::ColumnFamily::Assay,
            key,
            &report,
        )?;
        Ok(json!(report))
    }

    fn requires_authn(&self) -> bool {
        true
    }
}

impl Tool for KernelTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.kernel",
            "build or read the grounding kernel",
            "build/get the grounding kernel + recall + grounding gaps",
            object_schema(&[
                ("vault", string_schema(), true),
                ("anchor", string_schema(), false),
                ("rebuild", boolean_schema(), false),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: KernelArgs = decode("calyx.kernel", params)?;
        let ctx = core::load_context(&args.vault)?;
        let docs = core::load_docs(&ctx.vault)?;
        let anchor = args.anchor.as_deref().map(core::parse_anchor).transpose()?;
        let label = anchor.as_ref().map(core::anchor_label);
        let key = model::kernel_key(label.as_deref());
        let report = metrics::kernel(&docs, anchor.as_ref())?;
        if args.rebuild.unwrap_or(false)
            || !core::row_exists(&ctx.vault, ColumnFamily::Kernel, &key)?
        {
            core::write_json_row(&ctx.vault, ColumnFamily::Kernel, key, &report)?;
        }
        Ok(json!(report))
    }

    fn requires_authn(&self) -> bool {
        true
    }
}

impl Tool for GuardCalibrateTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.guard.calibrate",
            "calibrate a Gtau boundary",
            "calibrate the Gτ boundary for a domain",
            object_schema(&[
                ("vault", string_schema(), true),
                ("domain", string_schema(), true),
                ("set", string_schema(), true),
                ("target_far", number_schema(), true),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: GuardCalibrateArgs = decode("calyx.guard.calibrate", params)?;
        if !args.target_far.is_finite() {
            return Err(ToolError::invalid_params("target_far must be finite"));
        }
        guard::calibrate(&args.vault, &args.domain, &args.set, args.target_far)
    }

    fn requires_authn(&self) -> bool {
        true
    }
}

impl Tool for GuardCheckTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.guard.check",
            "apply a calibrated Gtau boundary",
            "apply the Gτ boundary to a constellation or text",
            object_schema(&[
                ("vault", string_schema(), true),
                ("cx_id", string_schema(), false),
                ("text", string_schema(), false),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: GuardCheckArgs = decode("calyx.guard.check", params)?;
        guard::check(&args.vault, args.cx_id.as_deref(), args.text.as_deref())
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

impl Tool for ProposeLensTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.propose_lens",
            "propose a lens to close an intelligence gap",
            "ask Calyx what lens would close a sufficiency gap",
            object_schema(&[
                ("vault", string_schema(), true),
                ("anchor", string_schema(), true),
                ("max_vram_mb", number_schema(), false),
                ("max_ram_mb", number_schema(), false),
                ("max_ms_per_input", number_schema(), false),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: ProposeLensArgs = decode("calyx.propose_lens", params)?;
        propose::run(&args.vault, &args.anchor, args.panel_budget()?)
    }

    fn requires_authn(&self) -> bool {
        true
    }
}

#[derive(Deserialize)]
struct VaultArgs {
    vault: String,
}

#[derive(Deserialize)]
struct BitsArgs {
    vault: String,
    anchor: String,
    explain: Option<bool>,
}

#[derive(Deserialize)]
struct KernelArgs {
    vault: String,
    anchor: Option<String>,
    rebuild: Option<bool>,
}

#[derive(Deserialize)]
struct GuardCalibrateArgs {
    vault: String,
    domain: String,
    set: PathBuf,
    target_far: f32,
}

#[derive(Deserialize)]
struct GuardCheckArgs {
    vault: String,
    cx_id: Option<String>,
    text: Option<String>,
}

#[derive(Deserialize)]
struct ProposeLensArgs {
    vault: String,
    anchor: String,
    max_vram_mb: Option<f64>,
    max_ram_mb: Option<f64>,
    max_ms_per_input: Option<f64>,
}

impl ProposeLensArgs {
    fn panel_budget(&self) -> ToolResult<Option<PanelResourceBudget>> {
        match (self.max_vram_mb, self.max_ram_mb, self.max_ms_per_input) {
            (None, None, None) => Ok(None),
            (Some(vram), Some(ram), Some(ms)) => Ok(Some(PanelResourceBudget {
                max_vram_mb: finite_f32("max_vram_mb", vram)?,
                max_ram_mb: finite_f32("max_ram_mb", ram)?,
                max_ms_per_input: finite_f32("max_ms_per_input", ms)?,
            })),
            _ => Err(ToolError::invalid_params(
                "max_vram_mb, max_ram_mb, and max_ms_per_input must be supplied together",
            )),
        }
    }
}

fn finite_f32(field: &'static str, value: f64) -> ToolResult<f32> {
    if !value.is_finite() || value < 0.0 || value > f64::from(f32::MAX) {
        return Err(ToolError::invalid_params(format!(
            "{field} must be a finite non-negative f32"
        )));
    }
    Ok(value as f32)
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
