use std::path::{Path, PathBuf};

use calyx_core::CalyxError;
use serde::Serialize;

use super::catalog::{LensCatalog, LensCatalogEntry, catalog_path, read_catalog, write_catalog};
use super::flags::value;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const CALYX_LENS_CATALOG_REMOVE_SELECTOR: &str = "CALYX_LENS_CATALOG_REMOVE_SELECTOR";
const CALYX_LENS_CATALOG_REMOVE_MISSING: &str = "CALYX_LENS_CATALOG_REMOVE_MISSING";
const CALYX_LENS_CATALOG_REMOVE_AMBIGUOUS: &str = "CALYX_LENS_CATALOG_REMOVE_AMBIGUOUS";
const REMOVE_REMEDIATION: &str =
    "pass exactly one existing --name or --lens-id and rerun calyx lens remove";

#[derive(Debug, Default)]
struct RemoveFlags {
    home: Option<PathBuf>,
    name: Option<String>,
    lens_id: Option<String>,
}

#[derive(Clone, Debug)]
enum RemoveSelector {
    Name(String),
    LensId(String),
}

#[derive(Debug, Serialize)]
struct RemoveReport {
    catalog: PathBuf,
    selector: RemoveSelectorReport,
    removed: LensCatalogEntry,
    before_count: usize,
    after_count: usize,
    before_vram_bytes: u64,
    after_vram_bytes: u64,
    freed_vram_bytes: u64,
    before_ram_bytes: u64,
    after_ram_bytes: u64,
    freed_ram_bytes: u64,
}

#[derive(Debug, Serialize)]
struct RemoveSelectorReport {
    kind: &'static str,
    value: String,
}

pub(crate) fn remove(args: &[String]) -> CliResult {
    let flags = RemoveFlags::parse(args)?;
    let selector = flags.selector()?;
    let report = remove_from_catalog(flags.home.as_deref(), selector)?;
    print_json(&report)
}

fn remove_from_catalog(home: Option<&Path>, selector: RemoveSelector) -> CliResult<RemoveReport> {
    let catalog = catalog_path(home)?;
    let mut state = read_catalog(&catalog)?;
    let before_count = state.lenses.len();
    let before_vram_bytes = placed_vram_bytes(&state);
    let before_ram_bytes = cpu_ram_bytes(&state);
    let removed = remove_entry(&mut state, &selector)?;
    let after_count = state.lenses.len();
    let after_vram_bytes = placed_vram_bytes(&state);
    let after_ram_bytes = cpu_ram_bytes(&state);

    write_catalog(&catalog, &state)?;
    Ok(RemoveReport {
        catalog,
        selector: selector.report(),
        removed,
        before_count,
        after_count,
        before_vram_bytes,
        after_vram_bytes,
        freed_vram_bytes: before_vram_bytes.saturating_sub(after_vram_bytes),
        before_ram_bytes,
        after_ram_bytes,
        freed_ram_bytes: before_ram_bytes.saturating_sub(after_ram_bytes),
    })
}

fn remove_entry(
    catalog: &mut LensCatalog,
    selector: &RemoveSelector,
) -> CliResult<LensCatalogEntry> {
    let matches: Vec<usize> = catalog
        .lenses
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| selector.matches(entry).then_some(idx))
        .collect();

    match matches.as_slice() {
        [] => Err(remove_error(
            CALYX_LENS_CATALOG_REMOVE_MISSING,
            format!("no lens catalog entry matches {}", selector.describe()),
        )),
        [idx] => Ok(catalog.lenses.remove(*idx)),
        _ => Err(remove_error(
            CALYX_LENS_CATALOG_REMOVE_AMBIGUOUS,
            format!(
                "{} matched {} catalog entries",
                selector.describe(),
                matches.len()
            ),
        )),
    }
}

fn placed_vram_bytes(catalog: &LensCatalog) -> u64 {
    catalog
        .lenses
        .iter()
        .filter(|entry| entry.placement == calyx_core::Placement::Gpu)
        .map(|entry| entry.cost.vram_bytes)
        .fold(0_u64, u64::saturating_add)
}

fn cpu_ram_bytes(catalog: &LensCatalog) -> u64 {
    catalog
        .lenses
        .iter()
        .filter(|entry| entry.placement == calyx_core::Placement::Cpu)
        .map(|entry| entry.cost.ram_bytes)
        .fold(0_u64, u64::saturating_add)
}

fn remove_error(code: &'static str, message: String) -> CliError {
    CliError::Calyx(CalyxError {
        code,
        message,
        remediation: REMOVE_REMEDIATION,
    })
}

impl RemoveFlags {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut flags = Self::default();
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--home" => {
                    idx += 1;
                    flags.home = Some(value(args, idx, "--home")?.into());
                }
                "--name" => {
                    idx += 1;
                    if flags
                        .name
                        .replace(value(args, idx, "--name")?.to_string())
                        .is_some()
                    {
                        return Err(remove_error(
                            CALYX_LENS_CATALOG_REMOVE_SELECTOR,
                            "--name was provided more than once".to_string(),
                        ));
                    }
                }
                "--lens-id" => {
                    idx += 1;
                    if flags
                        .lens_id
                        .replace(value(args, idx, "--lens-id")?.to_string())
                        .is_some()
                    {
                        return Err(remove_error(
                            CALYX_LENS_CATALOG_REMOVE_SELECTOR,
                            "--lens-id was provided more than once".to_string(),
                        ));
                    }
                }
                other => {
                    return Err(CliError::usage(format!(
                        "unexpected lens remove flag {other}"
                    )));
                }
            }
            idx += 1;
        }
        Ok(flags)
    }

    fn selector(&self) -> CliResult<RemoveSelector> {
        match (&self.name, &self.lens_id) {
            (Some(name), None) => Ok(RemoveSelector::Name(name.clone())),
            (None, Some(lens_id)) => Ok(RemoveSelector::LensId(lens_id.clone())),
            (None, None) => Err(remove_error(
                CALYX_LENS_CATALOG_REMOVE_SELECTOR,
                "calyx lens remove requires --name <name> or --lens-id <id>".to_string(),
            )),
            (Some(_), Some(_)) => Err(remove_error(
                CALYX_LENS_CATALOG_REMOVE_SELECTOR,
                "calyx lens remove accepts exactly one of --name or --lens-id".to_string(),
            )),
        }
    }
}

impl RemoveSelector {
    fn matches(&self, entry: &LensCatalogEntry) -> bool {
        match self {
            Self::Name(name) => entry.name == *name,
            Self::LensId(lens_id) => entry.lens_id == *lens_id,
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Name(name) => format!("name `{name}`"),
            Self::LensId(lens_id) => format!("lens_id `{lens_id}`"),
        }
    }

    fn report(self) -> RemoveSelectorReport {
        match self {
            Self::Name(value) => RemoveSelectorReport {
                kind: "name",
                value,
            },
            Self::LensId(value) => RemoveSelectorReport {
                kind: "lens_id",
                value,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use calyx_core::{LensCost, Placement};

    use super::*;

    #[test]
    fn remove_by_name_deletes_exact_row_and_reports_budget_delta() {
        let root = temp_root("remove-by-name");
        write_fixture_catalog(
            &root,
            vec![
                entry("keep-id", "keep", Placement::Cpu, 0, 11),
                entry("drop-id", "drop", Placement::Gpu, 17, 17),
            ],
        );

        let report =
            remove_from_catalog(Some(&root), RemoveSelector::Name("drop".to_string())).unwrap();
        let stored = read_catalog(&catalog_path(Some(&root)).unwrap()).unwrap();

        assert_eq!(report.removed.lens_id, "drop-id");
        assert_eq!(report.before_count, 2);
        assert_eq!(report.after_count, 1);
        assert_eq!(report.freed_vram_bytes, 17);
        assert_eq!(stored.lenses.len(), 1);
        assert_eq!(stored.lenses[0].name, "keep");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remove_by_lens_id_deletes_exact_row() {
        let root = temp_root("remove-by-id");
        write_fixture_catalog(
            &root,
            vec![
                entry("keep-id", "keep", Placement::Cpu, 0, 11),
                entry("drop-id", "drop", Placement::Cpu, 0, 23),
            ],
        );

        let report =
            remove_from_catalog(Some(&root), RemoveSelector::LensId("drop-id".to_string()))
                .unwrap();

        assert_eq!(report.removed.name, "drop");
        assert_eq!(report.freed_ram_bytes, 23);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_selector_fails_without_writing_catalog() {
        let root = temp_root("remove-missing");
        write_fixture_catalog(&root, vec![entry("keep-id", "keep", Placement::Gpu, 5, 5)]);
        let before = fs::read(catalog_path(Some(&root)).unwrap()).unwrap();

        let error = remove_from_catalog(Some(&root), RemoveSelector::Name("absent".to_string()))
            .unwrap_err();
        let after = fs::read(catalog_path(Some(&root)).unwrap()).unwrap();

        assert_eq!(error.code(), CALYX_LENS_CATALOG_REMOVE_MISSING);
        assert_eq!(before, after);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ambiguous_selector_fails_without_writing_catalog() {
        let root = temp_root("remove-ambiguous");
        write_fixture_catalog(
            &root,
            vec![
                entry("left-id", "same", Placement::Gpu, 5, 5),
                entry("right-id", "same", Placement::Gpu, 7, 7),
            ],
        );
        let before = fs::read(catalog_path(Some(&root)).unwrap()).unwrap();

        let error =
            remove_from_catalog(Some(&root), RemoveSelector::Name("same".to_string())).unwrap_err();
        let after = fs::read(catalog_path(Some(&root)).unwrap()).unwrap();

        assert_eq!(error.code(), CALYX_LENS_CATALOG_REMOVE_AMBIGUOUS);
        assert_eq!(before, after);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn selector_requires_exactly_one_identity_flag() {
        let no_selector = RemoveFlags::default().selector().unwrap_err();
        let both = RemoveFlags {
            name: Some("name".to_string()),
            lens_id: Some("id".to_string()),
            home: None,
        }
        .selector()
        .unwrap_err();

        assert_eq!(no_selector.code(), CALYX_LENS_CATALOG_REMOVE_SELECTOR);
        assert_eq!(both.code(), CALYX_LENS_CATALOG_REMOVE_SELECTOR);
    }

    fn write_fixture_catalog(root: &Path, lenses: Vec<LensCatalogEntry>) {
        let catalog = LensCatalog { lenses };
        let path = catalog_path(Some(root)).unwrap();
        write_catalog(&path, &catalog).unwrap();
    }

    fn entry(
        lens_id: &str,
        name: &str,
        placement: Placement,
        vram_bytes: u64,
        ram_bytes: u64,
    ) -> LensCatalogEntry {
        LensCatalogEntry {
            lens_id: lens_id.to_string(),
            name: name.to_string(),
            modality: "text".to_string(),
            runtime: "onnx".to_string(),
            dim: 384,
            retrieval_only: false,
            excluded_from_dedup: false,
            weights_sha256: "00".repeat(32),
            manifest: PathBuf::from(format!("{name}.json")),
            cost: LensCost {
                total_ms: 0.0,
                ms_per_input: 0.0,
                vram_bytes,
                ram_bytes,
                batch_ceiling: u32::MAX,
            },
            placement,
        }
    }

    fn temp_root(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "calyx-lens-remove-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
