//! `calyx navigate <mode>` — engine navigation surface over the CLI (issue
//! #599, PRD 10 §4/§9, 14 §2).
//!
//! Brings the navigation primitives the engine already exposes
//! (`neighbors`/`define`/`agree`/`disagree`/`traverse`/`skills`/`search_skill`)
//! to the CLI so the stack is reachable without the MCP surface. Every
//! subcommand reads a deterministic [`spec::NavSpec`] from `--spec <file>`,
//! rehydrates the exact `SearchEngine` the engine-level ph63 FSV uses, runs the
//! requested primitive, and prints the readback JSON. With `--out <file>` the
//! readback is written to a Source-of-Truth file, re-read from disk, and its
//! BLAKE3 digest printed — never trusting the in-memory value. Errors fail
//! closed with the upstream `CALYX_*` code.

mod spec;

use std::collections::BTreeMap;

use calyx_core::{CxId, SlotId, SlotVector};
use calyx_sextant::{
    Query, SearchEngine, SkillParams, TraverseDirection, agree, define, disagree, neighbors,
    search_skill, skills, traverse,
};
use serde_json::{Value, json};

use spec::{NavSpec, build_engine};

/// Dispatches a `calyx navigate <mode> [flags]` invocation.
pub fn run(mode: &str, args: &[String]) -> crate::error::CliResult {
    let flags = Flags::parse(args)?;
    match mode {
        "neighbors" => run_neighbors(&flags),
        "define" => run_define(&flags),
        "agree" => run_consensus(&flags, Consensus::Agree),
        "disagree" => run_consensus(&flags, Consensus::Disagree),
        "traverse" => run_traverse(&flags),
        "skills" => run_skills(&flags),
        "search-skill" => run_search_skill(&flags),
        other => Err(format!(
            "unknown navigate mode `{other}` (expected one of: neighbors, define, agree, \
             disagree, traverse, skills, search-skill)"
        )
        .into()),
    }
}

enum Consensus {
    Agree,
    Disagree,
}

fn run_neighbors(flags: &Flags) -> crate::error::CliResult {
    let engine = load_engine(flags)?;
    let cx = flags.cx_id("cx")?;
    let slot = flags.slot("slot")?;
    let k = flags.usize_value("k")?;
    let hits = neighbors(&engine, cx, slot, k).map_err(code_message)?;
    emit(
        json!({
            "mode": "neighbors",
            "cx": cx.to_string(),
            "slot": slot.get(),
            "k": k,
            "hits": hits,
        }),
        flags.optional("out"),
    )
}

fn run_define(flags: &Flags) -> crate::error::CliResult {
    let engine = load_engine(flags)?;
    let cx = flags.cx_id("cx")?;
    let slot = flags.slot("slot")?;
    let k = flags.usize_value("k")?;
    let constellation = define(&engine, cx, slot, k).map_err(code_message)?;
    emit(
        json!({
            "mode": "define",
            "cx": cx.to_string(),
            "slot": slot.get(),
            "k": k,
            "constellation": constellation,
        }),
        flags.optional("out"),
    )
}

fn run_consensus(flags: &Flags, kind: Consensus) -> crate::error::CliResult {
    let engine = load_engine(flags)?;
    let anchor = flags.cx_id("anchor")?;
    let k = flags.usize_value("k")?;
    let slot_filter = flags.slots("slots")?;
    let slot_filter_json = slot_filter
        .as_ref()
        .map(|slots| slots.iter().map(|slot| slot.get()).collect::<Vec<_>>());
    let (mode, report) = match kind {
        Consensus::Agree => ("agree", agree(&engine, anchor, k, slot_filter.as_deref())),
        Consensus::Disagree => (
            "disagree",
            disagree(&engine, anchor, k, slot_filter.as_deref()),
        ),
    };
    let report = report.map_err(code_message)?;
    emit(
        json!({
            "mode": mode,
            "anchor": anchor.to_string(),
            "k": k,
            "slot_filter": slot_filter_json,
            "report": report,
        }),
        flags.optional("out"),
    )
}

fn run_traverse(flags: &Flags) -> crate::error::CliResult {
    let engine = load_engine(flags)?;
    let anchor = flags.cx_id("anchor")?;
    let hops = u32::try_from(flags.usize_value("hops")?)
        .map_err(|_| "--hops is out of range".to_string())?;
    let direction = flags.direction("direction")?;
    let path = traverse(&engine, anchor, direction, hops).map_err(code_message)?;
    emit(
        json!({
            "mode": "traverse",
            "anchor": anchor.to_string(),
            "direction": flags.required("direction")?,
            "hops": hops,
            "path": path,
        }),
        flags.optional("out"),
    )
}

fn run_skills(flags: &Flags) -> crate::error::CliResult {
    let engine = load_engine(flags)?;
    let params = flags.skill_params()?;
    let tree = skills(&engine, &params).map_err(code_message)?;
    emit(
        json!({
            "mode": "skills",
            "params": params,
            "tree": tree,
        }),
        flags.optional("out"),
    )
}

fn run_search_skill(flags: &Flags) -> crate::error::CliResult {
    let engine = load_engine(flags)?;
    let params = flags.skill_params()?;
    let tree = skills(&engine, &params).map_err(code_message)?;
    let skill = flags.required("skill")?.to_string();
    let slot = flags.slot("slot")?;
    let k = flags.usize_value("k")?;
    let text = flags.optional("text").unwrap_or("navigate-search-skill");
    let vector = flags.vector("vec")?;
    let query = Query::new(text)
        .with_vector(SlotVector::Dense {
            dim: vector.len() as u32,
            data: vector,
        })
        .with_slots(vec![slot]);
    let query = Query { k, ..query };
    let hits = search_skill(&engine, &tree, &skill, &query).map_err(code_message)?;
    emit(
        json!({
            "mode": "search-skill",
            "skill": skill,
            "slot": slot.get(),
            "k": k,
            "hits": hits,
        }),
        flags.optional("out"),
    )
}

/// Reads and rehydrates the `--spec` engine specification.
fn load_engine(flags: &Flags) -> Result<SearchEngine, String> {
    let path = flags.required("spec")?;
    let bytes = std::fs::read(path).map_err(|error| format!("read spec {path}: {error}"))?;
    let spec: NavSpec = serde_json::from_slice(&bytes)
        .map_err(|error| format!("CALYX_NAVIGATE_SPEC_INVALID: parse spec {path}: {error}"))?;
    build_engine(&spec)
}

/// Serializes the readback, optionally pinning it to a Source-of-Truth file.
fn emit(report: Value, out: Option<&str>) -> crate::error::CliResult {
    let bytes = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    match out {
        Some(path) => {
            std::fs::write(path, &bytes).map_err(|error| format!("write {path}: {error}"))?;
            // Re-read the bytes from disk and digest those, not the in-memory value.
            let reread = std::fs::read(path).map_err(|error| format!("reread {path}: {error}"))?;
            let digest = blake3::hash(&reread).to_hex();
            println!("{}", String::from_utf8_lossy(&reread));
            println!("NAVIGATE_OUT={path}");
            println!("NAVIGATE_BLAKE3={digest}");
        }
        None => println!("{}", String::from_utf8_lossy(&bytes)),
    }
    Ok(())
}

fn code_message(error: calyx_core::CalyxError) -> String {
    format!("{}: {}", error.code, error.message)
}

/// `--key value` / boolean `--flag` parser shared by every navigate mode.
struct Flags {
    map: BTreeMap<String, String>,
}

impl Flags {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut map = BTreeMap::new();
        let mut index = 0;
        while index < args.len() {
            let token = &args[index];
            let key = token
                .strip_prefix("--")
                .ok_or_else(|| format!("expected `--flag`, got `{token}`"))?;
            match args.get(index + 1) {
                Some(value) if !value.starts_with("--") => {
                    map.insert(key.to_string(), value.clone());
                    index += 2;
                }
                _ => {
                    map.insert(key.to_string(), "true".to_string());
                    index += 1;
                }
            }
        }
        Ok(Self { map })
    }

    fn required(&self, key: &str) -> Result<&str, String> {
        self.map
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("missing required --{key}"))
    }

    fn optional(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }

    fn flag(&self, key: &str) -> bool {
        self.map
            .get(key)
            .map(|value| value == "true")
            .unwrap_or(false)
    }

    fn cx_id(&self, key: &str) -> Result<CxId, String> {
        self.required(key)?
            .parse::<CxId>()
            .map_err(|error| format!("invalid --{key}: {error}"))
    }

    fn slot(&self, key: &str) -> Result<SlotId, String> {
        self.required(key)?
            .parse::<SlotId>()
            .map_err(|error| format!("invalid --{key}: {error}"))
    }

    fn usize_value(&self, key: &str) -> Result<usize, String> {
        self.required(key)?
            .parse::<usize>()
            .map_err(|error| format!("invalid --{key}: {error}"))
    }

    fn slots(&self, key: &str) -> Result<Option<Vec<SlotId>>, String> {
        match self.optional(key) {
            None => Ok(None),
            Some(raw) => {
                let mut slots = Vec::new();
                for part in raw.split(',').filter(|part| !part.is_empty()) {
                    slots.push(
                        part.parse::<SlotId>()
                            .map_err(|error| format!("invalid --{key} entry `{part}`: {error}"))?,
                    );
                }
                if slots.is_empty() {
                    return Err(format!("--{key} listed no slots"));
                }
                Ok(Some(slots))
            }
        }
    }

    fn vector(&self, key: &str) -> Result<Vec<f32>, String> {
        let raw = self.required(key)?;
        let mut values = Vec::new();
        for part in raw.split(',').filter(|part| !part.is_empty()) {
            values.push(
                part.parse::<f32>()
                    .map_err(|error| format!("invalid --{key} entry `{part}`: {error}"))?,
            );
        }
        if values.is_empty() {
            return Err(format!("--{key} listed no values"));
        }
        Ok(values)
    }

    fn direction(&self, key: &str) -> Result<TraverseDirection, String> {
        match self.required(key)? {
            "forward" => Ok(TraverseDirection::Forward),
            "backward" => Ok(TraverseDirection::Backward),
            "both" => Ok(TraverseDirection::Both),
            other => Err(format!(
                "invalid --{key} `{other}` (expected forward|backward|both)"
            )),
        }
    }

    /// Builds skill-clustering params from optional flags, defaulting to the
    /// small-vault settings the ph63 fixture uses (min_cluster_size=2,
    /// min_samples=1) so a seeded spec clusters deterministically.
    fn skill_params(&self) -> Result<SkillParams, String> {
        let mut params = SkillParams {
            min_cluster_size: 2,
            min_samples: 1,
            ..SkillParams::default()
        };
        if let Some(value) = self.optional("min-cluster-size") {
            params.min_cluster_size = value
                .parse()
                .map_err(|error| format!("invalid --min-cluster-size: {error}"))?;
        }
        if let Some(value) = self.optional("min-samples") {
            params.min_samples = value
                .parse()
                .map_err(|error| format!("invalid --min-samples: {error}"))?;
        }
        if let Some(value) = self.optional("max-constellations") {
            params.max_constellations = value
                .parse()
                .map_err(|error| format!("invalid --max-constellations: {error}"))?;
        }
        params.slots = self.slots("slots")?;
        params.allow_single_cluster = self.flag("allow-single");
        Ok(params)
    }
}
