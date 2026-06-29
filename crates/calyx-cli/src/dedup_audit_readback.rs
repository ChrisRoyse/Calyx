use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use calyx_aster::cf::{ColumnFamily, base_key, slot_key};
use calyx_aster::dedup::{ReversalToken, dedup_audit, dedup_undo};
use calyx_aster::vault::encode::{decode_constellation_base, decode_slot_vector};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, SlotId, SlotVector};
use serde_json::json;

use crate::cf_read::{hex_bytes, latest_cf_row, latest_cf_rows, vault_id_from_base};
use crate::error::{CliError, CliResult};

const CX_LIST_UNBOUNDED_ROW_LIMIT: usize = 100;

#[derive(Debug)]
struct CxListArgs {
    vault: PathBuf,
    cx_id: Option<CxId>,
    limit: Option<usize>,
    include_slots: bool,
    allow_unbounded: bool,
}

pub fn readback_dedup_audit(vault: &Path, cx_id: &str) -> crate::error::CliResult {
    let cx_id = CxId::from_str(cx_id).map_err(|error| format!("invalid --cx-id: {error}"))?;
    let vault_id = vault_id_from_base(vault)?;
    let store = AsterVault::open(
        vault,
        vault_id,
        b"calyx-dedup-audit-readback".to_vec(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let report = dedup_audit(&store, cx_id).map_err(|error| error.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    Ok(())
}

pub fn readback_dedup_undo(vault: &Path, token: &str) -> crate::error::CliResult {
    let token: ReversalToken =
        serde_json::from_str(token).map_err(|error| format!("invalid --token: {error}"))?;
    let vault_id = vault_id_from_base(vault)?;
    let store = AsterVault::open(
        vault,
        vault_id,
        b"calyx-dedup-audit-readback".to_vec(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let before = latest_cf_rows(vault, ColumnFamily::Base)?;
    let restored = dedup_undo(&store, &token).map_err(|error| error.to_string())?;
    store.flush().map_err(|error| error.to_string())?;
    let after = latest_cf_rows(vault, ColumnFamily::Base)?;
    let value = json!({
        "vault": vault.display().to_string(),
        "restored": restored,
        "base_rows_before": before.len(),
        "base_rows_after": after.len(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
    );
    Ok(())
}

pub fn readback_cx_list_args(rest: &[String]) -> CliResult {
    let args = parse_cx_list_args(rest)?;
    if let Some(cx_id) = args.cx_id {
        let key = base_key(cx_id);
        let value = latest_cf_row(&args.vault, ColumnFamily::Base, &key)?.ok_or_else(|| {
            CliError::usage(format!(
                "cx-list --cx-id {cx_id} was not found in {}",
                args.vault.display()
            ))
        })?;
        let rows = BTreeMap::from([(key, value)]);
        return render_cx_list(&args.vault, rows, args.include_slots);
    }

    let mut rows = latest_cf_rows(&args.vault, ColumnFamily::Base)?;
    if let Some(limit) = args.limit {
        rows = rows.into_iter().take(limit).collect();
    } else if args.cx_id.is_none()
        && rows.len() > CX_LIST_UNBOUNDED_ROW_LIMIT
        && !args.allow_unbounded
    {
        return Err(CliError::usage(format!(
            "cx-list would print {} rows from {}; use --cx-id <cx>, --limit <n>, or --allow-unbounded",
            rows.len(),
            args.vault.display()
        )));
    }
    let include_slots = args.include_slots || (args.cx_id.is_none() && args.limit.is_none());
    render_cx_list(&args.vault, rows, include_slots)
}

fn parse_cx_list_args(rest: &[String]) -> CliResult<CxListArgs> {
    let mut vault = None;
    let mut cx_id = None;
    let mut limit = None;
    let mut include_slots = false;
    let mut allow_unbounded = false;
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--vault" => {
                idx += 1;
                vault = Some(PathBuf::from(value(rest, idx, "--vault")?));
            }
            "--cx-id" => {
                idx += 1;
                let raw = value(rest, idx, "--cx-id")?;
                cx_id = Some(
                    CxId::from_str(raw)
                        .map_err(|error| CliError::usage(format!("invalid --cx-id: {error}")))?,
                );
            }
            "--limit" => {
                idx += 1;
                let raw = value(rest, idx, "--limit")?;
                let parsed = raw
                    .parse::<usize>()
                    .map_err(|error| CliError::usage(format!("invalid --limit {raw}: {error}")))?;
                if parsed == 0 {
                    return Err(CliError::usage("--limit must be at least 1"));
                }
                limit = Some(parsed);
            }
            "--allow-unbounded" => allow_unbounded = true,
            "--include-slots" => include_slots = true,
            other => return Err(CliError::usage(format!("unexpected cx-list flag {other}"))),
        }
        idx += 1;
    }
    Ok(CxListArgs {
        vault: vault.ok_or_else(|| CliError::usage("cx-list requires --vault <dir>"))?,
        cx_id,
        limit,
        include_slots,
        allow_unbounded,
    })
}

fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

fn render_cx_list(
    vault: &Path,
    rows: BTreeMap<Vec<u8>, Vec<u8>>,
    include_slots: bool,
) -> crate::error::CliResult {
    let mut values = Vec::new();
    let mut slot_cache = BTreeMap::<SlotId, BTreeMap<Vec<u8>, Vec<u8>>>::new();
    let mut raw_slot_cache = BTreeMap::<SlotId, BTreeMap<Vec<u8>, Vec<u8>>>::new();
    for (key, value) in rows {
        let cx = decode_constellation_base(&value).map_err(|error| error.to_string())?;
        let mut row = json!({
            "key_hex": hex_bytes(&key),
            "cx_id": cx.cx_id,
            "created_at": cx.created_at,
            "panel_version": cx.panel_version,
            "flags": cx.flags,
            "base_slot_count": cx.slots.len(),
            "base_hex": hex_bytes(&value),
        });
        if include_slots {
            let slots = decoded_slot_entries(vault, &mut slot_cache, &mut raw_slot_cache, &cx)?;
            row["slot_summary"] = slot_summary(slots.iter().map(|(_, vector, _)| vector));
            row["slots"] = json!(
                slots
                    .iter()
                    .map(|(slot, vector, source)| {
                        match vector {
                            SlotVector::Dense { dim, data } => json!({
                                "slot": slot.get(),
                                "kind": "dense",
                                "payload_source": source,
                                "dim": dim,
                                "values": data.len(),
                            }),
                            SlotVector::Sparse { dim, entries } => json!({
                                "slot": slot.get(),
                                "kind": "sparse",
                                "payload_source": source,
                                "dim": dim,
                                "entries": entries.len(),
                            }),
                            SlotVector::Multi { token_dim, tokens } => json!({
                                "slot": slot.get(),
                                "kind": "multi",
                                "payload_source": source,
                                "token_dim": token_dim,
                                "tokens": tokens.len(),
                            }),
                            SlotVector::Absent { reason } => json!({
                                "slot": slot.get(),
                                "kind": "absent",
                                "payload_source": source,
                                "reason": reason,
                            }),
                        }
                    })
                    .collect::<Vec<_>>()
            );
        }
        values.push(row);
    }
    let json = serde_json::to_string_pretty(&values).map_err(|error| error.to_string())?;
    write_stdout_line(&json)?;
    Ok(())
}

fn write_stdout_line(text: &str) -> crate::error::CliResult {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    write_line_allow_broken_pipe(&mut lock, text)
}

fn write_line_allow_broken_pipe<W: Write>(writer: &mut W, text: &str) -> crate::error::CliResult {
    match writer.write_all(text.as_bytes()) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
        Err(error) => return Err(crate::error::CliError::io(format!("write stdout: {error}"))),
    }
    match writer.write_all(b"\n") {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(crate::error::CliError::io(format!("write stdout: {error}"))),
    }
}

fn decoded_slot_entries(
    vault: &Path,
    slot_cache: &mut BTreeMap<SlotId, BTreeMap<Vec<u8>, Vec<u8>>>,
    raw_slot_cache: &mut BTreeMap<SlotId, BTreeMap<Vec<u8>, Vec<u8>>>,
    cx: &calyx_core::Constellation,
) -> Result<Vec<(SlotId, SlotVector, &'static str)>, String> {
    let key = slot_key(cx.cx_id);
    let mut out = Vec::with_capacity(cx.slots.len());
    for (slot, placeholder) in &cx.slots {
        if !slot_cache.contains_key(slot) {
            slot_cache.insert(*slot, latest_cf_rows(vault, ColumnFamily::slot(*slot))?);
        }
        let Some(value) = slot_cache.get(slot).and_then(|rows| rows.get(&key)) else {
            out.push((
                *slot,
                placeholder.clone(),
                "base_hash_placeholder_missing_slot_cf",
            ));
            continue;
        };
        match decode_slot_vector(value) {
            Ok(vector) => out.push((*slot, vector, "slot_cf")),
            Err(_) => {
                if !raw_slot_cache.contains_key(slot) {
                    raw_slot_cache
                        .insert(*slot, latest_cf_rows(vault, ColumnFamily::slot_raw(*slot))?);
                }
                let vector = raw_slot_cache
                    .get(slot)
                    .and_then(|rows| rows.get(&key))
                    .map(|raw| decode_slot_vector(raw).map_err(|error| error.to_string()))
                    .transpose()?;
                out.push(match vector {
                    Some(vector) => (*slot, vector, "slot_raw_cf"),
                    None => (
                        *slot,
                        placeholder.clone(),
                        "base_hash_placeholder_missing_raw_cf",
                    ),
                });
            }
        }
    }
    Ok(out)
}

fn slot_summary<'a>(vectors: impl Iterator<Item = &'a SlotVector>) -> serde_json::Value {
    let mut dense_slots = 0usize;
    let mut sparse_slots = 0usize;
    let mut multi_slots = 0usize;
    let mut absent_reasons = BTreeMap::<String, usize>::new();
    for vector in vectors {
        match vector {
            SlotVector::Dense { .. } => dense_slots += 1,
            SlotVector::Sparse { .. } => sparse_slots += 1,
            SlotVector::Multi { .. } => multi_slots += 1,
            SlotVector::Absent { reason } => {
                let key = serde_json::to_value(reason)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_else(|| format!("{reason:?}"));
                *absent_reasons.entry(key).or_insert(0) += 1;
            }
        }
    }
    json!({
        "slot_count": dense_slots + sparse_slots + multi_slots + absent_reasons.values().sum::<usize>(),
        "dense_slots": dense_slots,
        "sparse_slots": sparse_slots,
        "multi_slots": multi_slots,
        "absent_slots": absent_reasons.values().sum::<usize>(),
        "absent_reasons": absent_reasons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingWriter(io::ErrorKind);

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(self.0, "synthetic write failure"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn write_line_appends_newline_on_success() {
        let mut out = Vec::new();

        write_line_allow_broken_pipe(&mut out, "{\"ok\":true}").unwrap();

        assert_eq!(out, b"{\"ok\":true}\n");
    }

    #[test]
    fn write_line_treats_broken_pipe_as_clean_early_consumer_exit() {
        let mut out = FailingWriter(io::ErrorKind::BrokenPipe);

        write_line_allow_broken_pipe(&mut out, "large readback").unwrap();
    }

    #[test]
    fn write_line_surfaces_non_broken_pipe_write_errors() {
        let mut out = FailingWriter(io::ErrorKind::PermissionDenied);

        let err = write_line_allow_broken_pipe(&mut out, "large readback").unwrap_err();

        assert_eq!(err.code(), "CALYX_CLI_IO_ERROR");
        assert!(err.message().contains("write stdout"), "{}", err.message());
        assert!(
            err.message().contains("synthetic write failure"),
            "{}",
            err.message()
        );
    }

    #[test]
    fn cx_list_args_parse_bounded_filters() {
        let cx_id = "00000000000000000000000000000001";
        let args = parse_cx_list_args(&[
            "--vault".to_string(),
            "vault-dir".to_string(),
            "--cx-id".to_string(),
            cx_id.to_string(),
            "--limit".to_string(),
            "1".to_string(),
        ])
        .unwrap();

        assert_eq!(args.vault, PathBuf::from("vault-dir"));
        assert_eq!(args.cx_id.unwrap().to_string(), cx_id);
        assert_eq!(args.limit, Some(1));
        assert!(!args.include_slots);
        assert!(!args.allow_unbounded);
    }

    #[test]
    fn cx_list_rejects_zero_limit() {
        let err = parse_cx_list_args(&[
            "--vault".to_string(),
            "vault-dir".to_string(),
            "--limit".to_string(),
            "0".to_string(),
        ])
        .unwrap_err();

        assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
        assert!(err.message().contains("at least 1"));
    }
}
