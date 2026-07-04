use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use calyx_aster::plain_graph::{
    GraphCollectionGenerationReadback, GraphCollectionGenerationState,
    GraphCollectionGenerationStatus, PhysicalGraphCollectionLifecycle,
};
use serde::Serialize;

use super::vault::{home_dir, resolve_vault_info};
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GraphCollectionGenerationsArgs {
    pub vault: String,
    pub collection: Option<String>,
    pub home: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GraphCollectionStateArgs {
    pub vault: String,
    pub collection: String,
    pub generation: String,
    pub state: GraphCollectionGenerationStatus,
    pub command: String,
    pub reason: Option<String>,
    pub detail: BTreeMap<String, String>,
    pub home: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct GraphCollectionGenerationsReport {
    status: &'static str,
    vault: String,
    vault_id: String,
    vault_dir: String,
    collection_filter: Option<String>,
    counts_by_state: BTreeMap<String, usize>,
    generations: Vec<GraphCollectionGenerationReadback>,
}

#[derive(Debug, Serialize)]
struct GraphCollectionStateReport {
    status: &'static str,
    vault: String,
    vault_id: String,
    vault_dir: String,
    write_seq: Option<u64>,
    write_path: &'static str,
    generation: GraphCollectionGenerationReadback,
}

pub(crate) fn parse_graph_collection_generations(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("graph-collection-generations requires <vault>"))?
        .clone();
    let mut collection = None;
    let mut home = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--collection" => {
                idx += 1;
                collection = Some(value(rest, idx, "--collection")?.to_string());
            }
            "--home" => {
                idx += 1;
                home = Some(value(rest, idx, "--home")?.into());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected graph-collection-generations flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::GraphCollectionGenerations(
        GraphCollectionGenerationsArgs {
            vault,
            collection,
            home,
        },
    ))
}

pub(crate) fn parse_graph_collection_state(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("graph-collection-state requires <vault>"))?
        .clone();
    let mut collection = None;
    let mut generation = None;
    let mut state = None;
    let mut command = None;
    let mut reason = None;
    let mut detail = BTreeMap::new();
    let mut home = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--collection" => {
                idx += 1;
                collection = Some(value(rest, idx, "--collection")?.to_string());
            }
            "--generation" => {
                idx += 1;
                generation = Some(value(rest, idx, "--generation")?.to_string());
            }
            "--state" => {
                idx += 1;
                state = Some(parse_state(value(rest, idx, "--state")?)?);
            }
            "--command" => {
                idx += 1;
                command = Some(value(rest, idx, "--command")?.to_string());
            }
            "--reason" => {
                idx += 1;
                reason = Some(value(rest, idx, "--reason")?.to_string());
            }
            "--detail" => {
                idx += 1;
                let (key, val) = parse_detail(value(rest, idx, "--detail")?)?;
                detail.insert(key, val);
            }
            "--home" => {
                idx += 1;
                home = Some(value(rest, idx, "--home")?.into());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected graph-collection-state flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::GraphCollectionState(GraphCollectionStateArgs {
        vault,
        collection: collection.ok_or_else(|| {
            CliError::usage("graph-collection-state requires --collection <name>")
        })?,
        generation: generation
            .ok_or_else(|| CliError::usage("graph-collection-state requires --generation <id>"))?,
        state: state
            .ok_or_else(|| CliError::usage("graph-collection-state requires --state <state>"))?,
        command: command
            .ok_or_else(|| CliError::usage("graph-collection-state requires --command <name>"))?,
        reason,
        detail,
        home,
    }))
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    match command {
        Subcommand::GraphCollectionGenerations(args) => list(args),
        Subcommand::GraphCollectionState(args) => put(args),
        _ => unreachable!("non graph lifecycle command routed here"),
    }
}

fn list(args: GraphCollectionGenerationsArgs) -> CliResult {
    let home = args
        .home
        .as_deref()
        .map_or_else(home_dir, |p| Ok(p.to_path_buf()))?;
    let resolved = resolve_vault_info(&home, &args.vault)?;
    let physical = PhysicalGraphCollectionLifecycle::open_latest(&resolved.path)?;
    let mut generations = physical.list_states()?;
    if let Some(collection) = &args.collection {
        generations.retain(|row| row.state.collection == *collection);
    }
    generations.sort_by(|left, right| {
        left.state
            .collection
            .cmp(&right.state.collection)
            .then(left.state.generation.cmp(&right.state.generation))
    });
    let mut counts_by_state = BTreeMap::new();
    for row in &generations {
        *counts_by_state
            .entry(row.state.status.as_str().to_string())
            .or_insert(0) += 1;
    }
    print_json(&GraphCollectionGenerationsReport {
        status: "ok",
        vault: resolved.name,
        vault_id: resolved.vault_id.to_string(),
        vault_dir: resolved.path.display().to_string(),
        collection_filter: args.collection,
        counts_by_state,
        generations,
    })
}

fn put(args: GraphCollectionStateArgs) -> CliResult {
    let home = args
        .home
        .as_deref()
        .map_or_else(home_dir, |p| Ok(p.to_path_buf()))?;
    let resolved = resolve_vault_info(&home, &args.vault)?;
    let mut state = GraphCollectionGenerationState::new(
        args.collection,
        args.generation,
        args.state,
        args.command,
    );
    state.reason = args.reason;
    state.detail = args.detail;
    let mut physical = PhysicalGraphCollectionLifecycle::open_latest(&resolved.path)?;
    physical.put_state_physical(&state)?;
    drop(physical);
    let generation = read_back_generation(&resolved.path, &state.collection, &state.generation)?;
    print_json(&GraphCollectionStateReport {
        status: "ok",
        vault: resolved.name,
        vault_id: resolved.vault_id.to_string(),
        vault_dir: resolved.path.display().to_string(),
        write_seq: None,
        write_path: "physical_graph_cf",
        generation,
    })
}

fn read_back_generation(
    vault_dir: &Path,
    collection: &str,
    generation: &str,
) -> CliResult<GraphCollectionGenerationReadback> {
    let physical = PhysicalGraphCollectionLifecycle::open_latest(vault_dir)?;
    physical
        .list_states()?
        .into_iter()
        .find(|row| row.state.collection == collection && row.state.generation == generation)
        .ok_or_else(|| {
            CliError::runtime(format!(
                "graph collection lifecycle row missing after write: {collection}/{generation}"
            ))
        })
}

fn parse_state(value: &str) -> CliResult<GraphCollectionGenerationStatus> {
    match value {
        "writing" => Ok(GraphCollectionGenerationStatus::Writing),
        "accepted" => Ok(GraphCollectionGenerationStatus::Accepted),
        "failed" => Ok(GraphCollectionGenerationStatus::Failed),
        "tombstoned" => Ok(GraphCollectionGenerationStatus::Tombstoned),
        other => Err(CliError::usage(format!(
            "invalid --state {other}; expected writing|accepted|failed|tombstoned"
        ))),
    }
}

fn parse_detail(value: &str) -> CliResult<(String, String)> {
    let Some((key, val)) = value.split_once('=') else {
        return Err(CliError::usage("--detail requires k=v"));
    };
    Ok((key.to_string(), val.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_graph_collection_lifecycle_commands() {
        let list = parse_graph_collection_generations(&[
            "vault".into(),
            "--collection".into(),
            "biomed".into(),
            "--home".into(),
            "/home/calyx".into(),
        ])
        .unwrap();
        assert_eq!(
            list,
            Subcommand::GraphCollectionGenerations(GraphCollectionGenerationsArgs {
                vault: "vault".into(),
                collection: Some("biomed".into()),
                home: Some("/home/calyx".into()),
            })
        );

        let put = parse_graph_collection_state(&[
            "vault".into(),
            "--collection".into(),
            "biomed".into(),
            "--generation".into(),
            "g1".into(),
            "--state".into(),
            "tombstoned".into(),
            "--command".into(),
            "materialize".into(),
            "--reason".into(),
            "aborted".into(),
            "--detail".into(),
            "report=missing".into(),
        ])
        .unwrap();
        let Subcommand::GraphCollectionState(args) = put else {
            panic!("expected graph collection state");
        };
        assert_eq!(args.state, GraphCollectionGenerationStatus::Tombstoned);
        assert_eq!(args.detail["report"], "missing");
    }
}
