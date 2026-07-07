use std::path::{Path, PathBuf};

use crate::error::{CliError, CliResult};
use crate::partitioned_bench::rrf_plan::{self, DEFAULT_ASSOCIATION_KEY};

pub(crate) fn run(raw: &[String]) -> CliResult {
    let args = Args::parse(raw)?;
    let (mut record, source_readback) =
        rrf_plan::read(&args.from_cf_root, &args.from_plan_key).map_err(CliError::Calyx)?;
    let source_base_dir = record.base_dir.clone();
    for slot in &mut record.plan.slots {
        let old_name = slot.vault.file_name().ok_or_else(|| {
            CliError::usage(format!(
                "slot {} vault path has no final component: {}",
                slot.slot,
                slot.vault.display()
            ))
        })?;
        let new_vault = args.vault_root.join(old_name);
        require_dir(&new_vault, "remapped vault")?;
        require_file(&rrf_plan::resolve(&source_base_dir, &slot.corpus), "corpus")?;
        if args.queries_from_corpus {
            slot.queries = slot.corpus.clone();
        } else {
            require_file(
                &rrf_plan::resolve(&source_base_dir, &slot.queries),
                "queries",
            )?;
        }
        if let Some(query_start_row) = args.query_start_row {
            slot.query_start_row = query_start_row;
        }
        slot.vault = new_vault;
    }
    if let Some(base_dir) = args.base_dir {
        record.base_dir = base_dir;
    }
    record.imported_plan_sha256 =
        rrf_plan::plan_identity_sha256(&record.base_dir, &record.plan).map_err(CliError::Calyx)?;
    let written =
        rrf_plan::write(&args.cf_root, &args.plan_key, &record).map_err(CliError::Calyx)?;
    println!(
        "partitioned_rrf_plan_remap_db from_cf_root={} from_plan_key={} source_value_sha256={} cf_root={} plan_key={} slot_count={} vault_root={} queries_from_corpus={} query_start_row={} plan_sha256={} value_bytes={} value_sha256={} readback_matches={}",
        args.from_cf_root.display(),
        args.from_plan_key,
        source_readback.value_sha256,
        written.cf_root,
        written.association_key,
        record.plan.slots.len(),
        args.vault_root.display(),
        args.queries_from_corpus,
        args.query_start_row.unwrap_or(0),
        record.imported_plan_sha256,
        written.value_bytes,
        written.value_sha256,
        written.readback_matches
    );
    Ok(())
}

#[derive(Clone, Debug)]
struct Args {
    from_cf_root: PathBuf,
    from_plan_key: String,
    cf_root: PathBuf,
    plan_key: String,
    vault_root: PathBuf,
    base_dir: Option<PathBuf>,
    queries_from_corpus: bool,
    query_start_row: Option<u64>,
}

impl Args {
    fn parse(raw: &[String]) -> CliResult<Self> {
        let mut from_cf_root = None;
        let mut from_plan_key = DEFAULT_ASSOCIATION_KEY.to_string();
        let mut cf_root = None;
        let mut plan_key = DEFAULT_ASSOCIATION_KEY.to_string();
        let mut vault_root = None;
        let mut base_dir = None;
        let mut queries_from_corpus = false;
        let mut query_start_row = None;
        let mut it = raw.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--from-cf-root" => from_cf_root = Some(PathBuf::from(next()?)),
                "--from-plan-key" => from_plan_key = next()?,
                "--cf-root" => cf_root = Some(PathBuf::from(next()?)),
                "--association-key" | "--plan-key" => plan_key = next()?,
                "--vault-root" => vault_root = Some(PathBuf::from(next()?)),
                "--base-dir" => base_dir = Some(PathBuf::from(next()?)),
                "--queries-from-corpus" => queries_from_corpus = true,
                "--query-start-row" => {
                    query_start_row = Some(super::parse(&next()?, "--query-start-row")?)
                }
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        require_key("--from-plan-key", &from_plan_key)?;
        require_key("--plan-key", &plan_key)?;
        Ok(Self {
            from_cf_root: from_cf_root
                .ok_or_else(|| CliError::usage("--from-cf-root <aster-dir> is required"))?,
            from_plan_key,
            cf_root: cf_root.ok_or_else(|| CliError::usage("--cf-root <aster-dir> is required"))?,
            plan_key,
            vault_root: vault_root
                .ok_or_else(|| CliError::usage("--vault-root <dir> is required"))?,
            base_dir,
            queries_from_corpus,
            query_start_row,
        })
    }
}

fn require_key(flag: &'static str, value: &str) -> CliResult {
    if value.trim().is_empty() {
        return Err(CliError::usage(format!("{flag} must be non-empty")));
    }
    Ok(())
}

fn require_dir(path: &Path, label: &'static str) -> CliResult {
    if !path.is_dir() {
        return Err(CliError::usage(format!(
            "{label} path does not exist or is not a directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn require_file(path: &Path, label: &'static str) -> CliResult {
    if !path.is_file() {
        return Err(CliError::usage(format!(
            "{label} path does not exist or is not a file: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::partitioned_bench::rrf_plan::{
        FORMAT, MODE, PartitionedRrfPlanRecord, Plan, PlanSlot,
    };

    use super::*;

    #[test]
    fn remaps_vault_paths_from_db_to_db() {
        let root = temp_root("partitioned-rrf-plan-remap");
        let base = root.join("source-base");
        let old_vault = base.join("old-vault-a");
        let query = base.join("q.i8bin");
        let corpus = base.join("c.i8bin");
        let new_vault_root = root.join("new-vaults");
        let new_vault = new_vault_root.join("old-vault-a");
        fs::create_dir_all(&old_vault).unwrap();
        fs::create_dir_all(&new_vault).unwrap();
        fs::write(&query, b"query").unwrap();
        fs::write(&corpus, b"corpus").unwrap();
        let source_cf = root.join("source-cf");
        let dest_cf = root.join("dest-cf");
        rrf_plan::write(&source_cf, "source", &record(&base)).unwrap();

        run(&strings([
            "--from-cf-root",
            source_cf.to_str().unwrap(),
            "--from-plan-key",
            "source",
            "--cf-root",
            dest_cf.to_str().unwrap(),
            "--plan-key",
            "dest",
            "--vault-root",
            new_vault_root.to_str().unwrap(),
        ]))
        .unwrap();

        let (written, readback) = rrf_plan::read(&dest_cf, "dest").unwrap();
        assert!(readback.readback_matches);
        assert_eq!(written.base_dir, base);
        assert_eq!(written.plan.slots[0].vault, new_vault);
        assert_eq!(written.plan.slots[0].queries, PathBuf::from("q.i8bin"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remap_refuses_missing_vault_directory() {
        let root = temp_root("partitioned-rrf-plan-remap-missing-vault");
        let base = root.join("source-base");
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("q.i8bin"), b"query").unwrap();
        fs::write(base.join("c.i8bin"), b"corpus").unwrap();
        let source_cf = root.join("source-cf");
        let dest_cf = root.join("dest-cf");
        let new_vault_root = root.join("new-vaults");
        rrf_plan::write(&source_cf, "source", &record(&base)).unwrap();

        let err = run(&strings([
            "--from-cf-root",
            source_cf.to_str().unwrap(),
            "--from-plan-key",
            "source",
            "--cf-root",
            dest_cf.to_str().unwrap(),
            "--vault-root",
            new_vault_root.to_str().unwrap(),
        ]))
        .unwrap_err();

        assert!(err.message().contains("remapped vault path"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remap_can_use_corpus_as_query_source() {
        let root = temp_root("partitioned-rrf-plan-remap-corpus-query");
        let base = root.join("source-base");
        let old_vault = base.join("old-vault-a");
        let corpus = base.join("c.i8bin");
        let new_vault_root = root.join("new-vaults");
        let new_vault = new_vault_root.join("old-vault-a");
        fs::create_dir_all(&old_vault).unwrap();
        fs::create_dir_all(&new_vault).unwrap();
        fs::write(&corpus, b"corpus").unwrap();
        let source_cf = root.join("source-cf");
        let dest_cf = root.join("dest-cf");
        rrf_plan::write(&source_cf, "source", &record(&base)).unwrap();

        run(&strings([
            "--from-cf-root",
            source_cf.to_str().unwrap(),
            "--from-plan-key",
            "source",
            "--cf-root",
            dest_cf.to_str().unwrap(),
            "--plan-key",
            "dest",
            "--vault-root",
            new_vault_root.to_str().unwrap(),
            "--queries-from-corpus",
        ]))
        .unwrap();

        let (written, readback) = rrf_plan::read(&dest_cf, "dest").unwrap();
        assert!(readback.readback_matches);
        assert_eq!(written.plan.slots[0].queries, PathBuf::from("c.i8bin"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remap_persists_query_start_row_in_db_plan() {
        let root = temp_root("partitioned-rrf-plan-remap-query-start");
        let base = root.join("source-base");
        let old_vault = base.join("old-vault-a");
        let corpus = base.join("c.i8bin");
        let new_vault_root = root.join("new-vaults");
        let new_vault = new_vault_root.join("old-vault-a");
        fs::create_dir_all(&old_vault).unwrap();
        fs::create_dir_all(&new_vault).unwrap();
        fs::write(&corpus, b"corpus").unwrap();
        let source_cf = root.join("source-cf");
        let dest_cf = root.join("dest-cf");
        rrf_plan::write(&source_cf, "source", &record(&base)).unwrap();

        run(&strings([
            "--from-cf-root",
            source_cf.to_str().unwrap(),
            "--from-plan-key",
            "source",
            "--cf-root",
            dest_cf.to_str().unwrap(),
            "--plan-key",
            "dest",
            "--vault-root",
            new_vault_root.to_str().unwrap(),
            "--queries-from-corpus",
            "--query-start-row",
            "7",
        ]))
        .unwrap();

        let (written, _) = rrf_plan::read(&dest_cf, "dest").unwrap();
        assert_eq!(written.plan.slots[0].query_start_row, 7);
        assert_ne!(written.imported_plan_sha256, "00".repeat(32));
        let _ = fs::remove_dir_all(root);
    }

    fn record(base: &Path) -> PartitionedRrfPlanRecord {
        PartitionedRrfPlanRecord {
            format: FORMAT.to_string(),
            mode: MODE.to_string(),
            imported_plan_sha256: "00".repeat(32),
            base_dir: base.to_path_buf(),
            plan: Plan {
                timeline: None,
                slots: vec![PlanSlot {
                    slot: 0,
                    name: Some("unit".to_string()),
                    lens_id: Some("11".repeat(16)),
                    weights_sha256: Some("22".repeat(32)),
                    signal_kind: Some("algorithmic_structured".to_string()),
                    bits_about: Some(0.1),
                    vault: PathBuf::from("old-vault-a"),
                    queries: PathBuf::from("q.i8bin"),
                    query_start_row: 0,
                    corpus: PathBuf::from("c.i8bin"),
                }],
            },
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn strings<const N: usize>(items: [&str; N]) -> Vec<String> {
        items.into_iter().map(ToOwned::to_owned).collect()
    }
}
