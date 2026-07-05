use std::path::PathBuf;

use crate::error::{CliError, CliResult};
use crate::partitioned_bench::slot_truth_store::DEFAULT_ASSOCIATION_KEY;

const DEFAULT_CHUNK_ROWS: usize = 100_000;

#[derive(Clone, Debug)]
pub(super) struct Args {
    pub(super) plan: Option<PathBuf>,
    pub(super) plan_cf_root: Option<PathBuf>,
    pub(super) plan_key: String,
    pub(super) out_dir: Option<PathBuf>,
    pub(super) cf_root: Option<PathBuf>,
    pub(super) association_key: String,
    pub(super) query_count: usize,
    pub(super) truth_depth: usize,
    pub(super) chunk_rows: usize,
    pub(super) emit_artifacts: bool,
}

impl Args {
    pub(super) fn parse(raw: &[String]) -> CliResult<Self> {
        let mut plan = None;
        let mut plan_cf_root = None;
        let mut plan_key = crate::partitioned_bench::rrf_plan::DEFAULT_ASSOCIATION_KEY.to_string();
        let mut out_dir = None;
        let mut cf_root = None;
        let mut association_key = DEFAULT_ASSOCIATION_KEY.to_string();
        let mut query_count = None;
        let mut truth_depth = None;
        let mut chunk_rows = DEFAULT_CHUNK_ROWS;
        let mut emit_artifacts = true;
        let mut it = raw.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--plan" => plan = Some(PathBuf::from(next()?)),
                "--plan-cf-root" => plan_cf_root = Some(PathBuf::from(next()?)),
                "--plan-key" => plan_key = next()?,
                "--out-dir" => out_dir = Some(PathBuf::from(next()?)),
                "--cf-root" => cf_root = Some(PathBuf::from(next()?)),
                "--association-key" => association_key = next()?,
                "--query-count" => {
                    query_count = Some(super::super::parse(&next()?, "--query-count")?)
                }
                "--truth-depth" => {
                    truth_depth = Some(super::super::parse(&next()?, "--truth-depth")?)
                }
                "--chunk-rows" => chunk_rows = super::super::parse(&next()?, "--chunk-rows")?,
                "--db-only" | "--no-artifacts" => emit_artifacts = false,
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        let args = Self {
            plan,
            plan_cf_root,
            plan_key,
            out_dir,
            cf_root,
            association_key,
            query_count: query_count
                .ok_or_else(|| CliError::usage("--query-count <n> is required"))?,
            truth_depth: truth_depth
                .ok_or_else(|| CliError::usage("--truth-depth <n> is required"))?,
            chunk_rows,
            emit_artifacts,
        };
        args.validate()?;
        Ok(args)
    }

    fn validate(&self) -> CliResult {
        if self.query_count == 0 || self.truth_depth == 0 || self.chunk_rows == 0 {
            return Err(CliError::usage(
                "--query-count, --truth-depth, and --chunk-rows must be > 0",
            ));
        }
        if self.plan.is_some() == self.plan_cf_root.is_some() {
            return Err(CliError::usage(
                "pass exactly one of --plan <json> or --plan-cf-root <aster-dir>",
            ));
        }
        if self.plan_key.trim().is_empty() {
            return Err(CliError::usage("--plan-key must be non-empty"));
        }
        if self.emit_artifacts && self.out_dir.is_none() {
            return Err(CliError::usage(
                "--out-dir <dir> is required unless --db-only is set",
            ));
        }
        if !self.emit_artifacts && self.cf_root.is_none() {
            return Err(CliError::usage(
                "--cf-root <dir> is required with --db-only",
            ));
        }
        if self.association_key.trim().is_empty() {
            return Err(CliError::usage("--association-key must be non-empty"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn parses_db_only_mode() {
        let args = Args::parse(&strings([
            "--plan",
            "plan.json",
            "--cf-root",
            "slot-truth-db",
            "--association-key",
            "issue791_truth",
            "--query-count",
            "8",
            "--truth-depth",
            "16",
            "--db-only",
        ]))
        .unwrap();

        assert_eq!(args.plan, Some(PathBuf::from("plan.json")));
        assert_eq!(args.plan_cf_root, None);
        assert_eq!(args.cf_root, Some(PathBuf::from("slot-truth-db")));
        assert_eq!(args.association_key, "issue791_truth");
        assert!(!args.emit_artifacts);
    }

    #[test]
    fn db_only_requires_cf_root() {
        let err = Args::parse(&strings([
            "--plan",
            "plan.json",
            "--query-count",
            "8",
            "--truth-depth",
            "16",
            "--db-only",
        ]))
        .unwrap_err();

        assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
        assert!(err.message().contains("--cf-root <dir> is required"));
    }

    #[test]
    fn parses_plan_cf_root() {
        let args = Args::parse(&strings([
            "--plan-cf-root",
            "plan-db",
            "--plan-key",
            "issue791_plan",
            "--cf-root",
            "slot-truth-db",
            "--query-count",
            "8",
            "--truth-depth",
            "16",
            "--db-only",
        ]))
        .unwrap();

        assert_eq!(args.plan, None);
        assert_eq!(args.plan_cf_root, Some(PathBuf::from("plan-db")));
        assert_eq!(args.plan_key, "issue791_plan");
    }

    fn strings(items: impl IntoIterator<Item = &'static str>) -> Vec<String> {
        items.into_iter().map(str::to_string).collect()
    }
}
