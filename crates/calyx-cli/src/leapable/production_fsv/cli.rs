use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{PgConn, PgSnapshot, ProductionFSV, snapshot_pg_state};
use crate::error::{CliError, CliResult};
use crate::leapable::read_flip::text_query_vector;
use crate::output::print_json;

#[derive(Clone, Debug, PartialEq, Serialize)]
struct SnapshotReport {
    snapshot: PgSnapshot,
    gate: String,
}

pub(crate) fn run_production_fsv(args: &[String]) -> CliResult {
    let (command, rest) = args
        .split_first()
        .ok_or_else(|| CliError::usage("production-fsv requires a subcommand"))?;
    match command.as_str() {
        "snapshot-pg" => run_snapshot_pg(rest),
        "verify-pg-unchanged" => run_verify_pg(rest),
        "verify-contract" => run_verify_contract(rest),
        "run" => run_full(rest),
        other => Err(CliError::usage(format!(
            "unknown production-fsv subcommand {other}"
        ))),
    }
}

fn run_snapshot_pg(args: &[String]) -> CliResult {
    let args = SnapshotArgs::parse(args)?;
    let snapshot = snapshot_pg_state(&args.pg_conn, &args.vault_name, &args.out_tables)?;
    write_json(&args.out, &snapshot)?;
    print_json(&SnapshotReport {
        snapshot,
        gate: "PASS".to_string(),
    })
}

fn run_verify_pg(args: &[String]) -> CliResult {
    let args = VerifyPgArgs::parse(args)?;
    let before = read_snapshot(&args.before)?;
    let after = read_snapshot(&args.after)?;
    let proof = ProductionFSV::verify_pg_unchanged(&before, &after)?;
    print_json(&proof)
}

fn run_verify_contract(args: &[String]) -> CliResult {
    let args = VerifyContractArgs::parse(args)?;
    let snapshot = read_snapshot(&args.snapshot)?;
    let proof = ProductionFSV::verify_control_plane_contract(&args.vault_name, &snapshot)?;
    print_json(&proof)
}

fn run_full(args: &[String]) -> CliResult {
    let args = RunArgs::parse(args)?;
    let before_dir = args.out.with_extension("pg-before");
    let after_dir = args.out.with_extension("pg-after");
    let before = snapshot_pg_state(&args.pg_conn, &args.vault_name, &before_dir)?;
    let query_vec = match args.query {
        QueryArg::Vector(vector) => vector,
        QueryArg::Text(text) => text_query_vector(&text, args.query_dim),
    };
    let ask = ProductionFSV::run_full_ask_cycle(&args.vault, &query_vec, args.top_k)?;
    let after = snapshot_pg_state(&args.pg_conn, &args.vault_name, &after_dir)?;
    let bundle = ProductionFSV::bundle(&args.vault, before, after, ask)?;
    ProductionFSV::emit_evidence(&args.out, &bundle)?;
    print_json(&bundle)
}

fn read_snapshot(path: &Path) -> CliResult<PgSnapshot> {
    let bytes =
        fs::read(path).map_err(|err| CliError::io(format!("read {}: {err}", path.display())))?;
    serde_json::from_slice(&bytes)
        .map_err(|err| CliError::usage(format!("parse {}: {err}", path.display())))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| CliError::io(format!("create {}: {err}", parent.display())))?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|err| CliError::usage(format!("encode {}: {err}", path.display())))?;
    fs::write(path, bytes).map_err(|err| CliError::io(format!("write {}: {err}", path.display())))
}

#[derive(Clone, Debug)]
struct SnapshotArgs {
    vault_name: String,
    pg_conn: PgConn,
    out: PathBuf,
    out_tables: PathBuf,
}

impl SnapshotArgs {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault_name = None;
        let mut pg_conn = None;
        let mut pg_dump_dir = None;
        let mut out = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--vault-name" => vault_name = Some(value(args, idx, "--vault-name")?.to_string()),
                "--pg-conn" => {
                    pg_conn = Some(PgConn::ReadOnlyPsql {
                        conninfo: value(args, idx, "--pg-conn")?.to_string(),
                    })
                }
                "--pg-dump-dir" => {
                    pg_dump_dir = Some(PathBuf::from(value(args, idx, "--pg-dump-dir")?))
                }
                "--out" => out = Some(PathBuf::from(value(args, idx, "--out")?)),
                other => return Err(CliError::usage(format!("unknown snapshot-pg arg {other}"))),
            }
            idx += 2;
        }
        let out = out.ok_or_else(|| CliError::usage("snapshot-pg requires --out <json>"))?;
        let pg_conn = match (pg_conn, pg_dump_dir) {
            (Some(conn), None) => conn,
            (None, Some(root)) => PgConn::DumpDir { root },
            _ => {
                return Err(CliError::usage(
                    "snapshot-pg requires exactly one of --pg-conn or --pg-dump-dir",
                ));
            }
        };
        Ok(Self {
            vault_name: vault_name
                .ok_or_else(|| CliError::usage("snapshot-pg requires --vault-name <name>"))?,
            pg_conn,
            out_tables: out.with_extension("tables"),
            out,
        })
    }
}

#[derive(Clone, Debug)]
struct VerifyPgArgs {
    before: PathBuf,
    after: PathBuf,
}

impl VerifyPgArgs {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut before = None;
        let mut after = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--before" => before = Some(PathBuf::from(value(args, idx, "--before")?)),
                "--after" => after = Some(PathBuf::from(value(args, idx, "--after")?)),
                other => {
                    return Err(CliError::usage(format!(
                        "unknown verify-pg-unchanged arg {other}"
                    )));
                }
            }
            idx += 2;
        }
        Ok(Self {
            before: before
                .ok_or_else(|| CliError::usage("verify-pg-unchanged requires --before"))?,
            after: after.ok_or_else(|| CliError::usage("verify-pg-unchanged requires --after"))?,
        })
    }
}

#[derive(Clone, Debug)]
struct VerifyContractArgs {
    vault_name: String,
    snapshot: PathBuf,
}

impl VerifyContractArgs {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault_name = None;
        let mut snapshot = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--vault-name" => vault_name = Some(value(args, idx, "--vault-name")?.to_string()),
                "--snapshot" => snapshot = Some(PathBuf::from(value(args, idx, "--snapshot")?)),
                other => {
                    return Err(CliError::usage(format!(
                        "unknown verify-contract arg {other}"
                    )));
                }
            }
            idx += 2;
        }
        Ok(Self {
            vault_name: vault_name
                .ok_or_else(|| CliError::usage("verify-contract requires --vault-name"))?,
            snapshot: snapshot
                .ok_or_else(|| CliError::usage("verify-contract requires --snapshot"))?,
        })
    }
}

#[derive(Clone, Debug)]
struct RunArgs {
    vault: PathBuf,
    vault_name: String,
    pg_conn: PgConn,
    query: QueryArg,
    query_dim: usize,
    top_k: usize,
    out: PathBuf,
}

impl RunArgs {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault = None;
        let mut vault_name = None;
        let mut pg_conn = None;
        let mut pg_dump_dir = None;
        let mut query = None;
        let mut query_dim = 768usize;
        let mut top_k = 5usize;
        let mut out = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--vault" => vault = Some(PathBuf::from(value(args, idx, "--vault")?)),
                "--vault-name" => vault_name = Some(value(args, idx, "--vault-name")?.to_string()),
                "--pg-conn" => {
                    pg_conn = Some(PgConn::ReadOnlyPsql {
                        conninfo: value(args, idx, "--pg-conn")?.to_string(),
                    })
                }
                "--pg-dump-dir" => {
                    pg_dump_dir = Some(PathBuf::from(value(args, idx, "--pg-dump-dir")?))
                }
                "--query-vector" => {
                    query = Some(QueryArg::Vector(parse_vector(value(
                        args,
                        idx,
                        "--query-vector",
                    )?)?))
                }
                "--query" => query = Some(QueryArg::Text(value(args, idx, "--query")?.to_string())),
                "--query-dim" => {
                    query_dim = value(args, idx, "--query-dim")?
                        .parse()
                        .map_err(|err| CliError::usage(format!("parse --query-dim: {err}")))?;
                }
                "--top-k" => {
                    top_k = value(args, idx, "--top-k")?
                        .parse()
                        .map_err(|err| CliError::usage(format!("parse --top-k: {err}")))?;
                }
                "--out" => out = Some(PathBuf::from(value(args, idx, "--out")?)),
                other => {
                    return Err(CliError::usage(format!(
                        "unknown production-fsv run arg {other}"
                    )));
                }
            }
            idx += 2;
        }
        let pg_conn = match (pg_conn, pg_dump_dir) {
            (Some(conn), None) => conn,
            (None, Some(root)) => PgConn::DumpDir { root },
            _ => {
                return Err(CliError::usage(
                    "run requires exactly one of --pg-conn or --pg-dump-dir",
                ));
            }
        };
        Ok(Self {
            vault: vault.ok_or_else(|| CliError::usage("run requires --vault <dir>"))?,
            vault_name: vault_name.ok_or_else(|| CliError::usage("run requires --vault-name"))?,
            pg_conn,
            query: query.ok_or_else(|| {
                CliError::usage("run requires --query-vector <json-array> or --query <text>")
            })?,
            query_dim,
            top_k,
            out: out.ok_or_else(|| CliError::usage("run requires --out <json>"))?,
        })
    }
}

#[derive(Clone, Debug)]
enum QueryArg {
    Vector(Vec<f32>),
    Text(String),
}

fn parse_vector(value: &str) -> CliResult<Vec<f32>> {
    serde_json::from_str(value)
        .map_err(|err| CliError::usage(format!("parse --query-vector JSON array: {err}")))
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> CliResult<&'a str> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}
