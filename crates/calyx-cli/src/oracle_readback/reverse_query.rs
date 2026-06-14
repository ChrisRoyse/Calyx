use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use calyx_aster::cf::{ColumnFamily, base_key, ledger_key, recurrence_key};
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::{
    Occurrence, OccurrenceContext, StoredRecurrenceRow, encode_recurrence_row,
};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
use calyx_core::{
    AnchorValue, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, VaultId,
    content_address,
};
use calyx_oracle::{
    DomainId, MAX_REVERSE_DEPTH, ORACLE_ACTION_METADATA_KEY, ORACLE_DOMAIN_METADATA_KEY,
    ORACLE_EFFECT_METADATA_KEY, ORACLE_STRUCTURAL_CONFIDENCE_METADATA_KEY, reverse_query,
};
use serde::Deserialize;
use serde_json::json;

use crate::cf_read::hex_bytes;

const USAGE: &str = "usage: calyx readback reverse_query --vault <dir> --domain <domain> --answer <text> --fixture <json> --vault-id <id> --salt <s>";

pub(crate) fn readback_reverse_query(args: &[String]) -> crate::error::CliResult {
    let args = ReadbackArgs::parse(args)?;
    let fixture = ReverseFixture::read(&args.fixture, &args.domain)?;
    let vault_id = VaultId::from_str(&args.vault_id)
        .map_err(|error| format!("invalid --vault-id: {error}"))?;
    let vault = AsterVault::new_durable(
        &args.vault,
        vault_id,
        args.salt.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let rows = fixture.persist_rows(&vault, vault_id, &args.domain)?;
    vault
        .flush()
        .map_err(|error| format!("flush reverse_query fixture rows: {error}"))?;

    let answer = AnchorValue::Text(args.answer.clone());
    let clock = FixedClock::new(fixture.clock_ts);
    match reverse_query(&vault, &answer, DomainId::from(args.domain.clone()), &clock) {
        Ok(causes) => {
            vault
                .flush()
                .map_err(|error| format!("flush reverse_query ledger row: {error}"))?;
            let ledger_ref = causes.first().map(|cause| cause.provenance.clone());
            let ledger_row = ledger_ref
                .as_ref()
                .map(|ref_| read_ledger_row(&vault, ref_))
                .transpose()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "operation": "calyx readback reverse_query",
                    "source_of_truth": {
                        "vault": args.vault,
                        "base_rows_written": rows.base_rows,
                        "recurrence_rows_written": rows.recurrence_rows,
                        "ledger_ref": ledger_ref,
                        "ledger_key_hex": ledger_ref.as_ref().map(|ref_| hex_bytes(&ledger_key(ref_.seq))),
                        "ledger_value_b3": ledger_row.as_ref().map(|row| hex_bytes(blake3::hash(row).as_bytes())),
                        "ledger_value_len": ledger_row.as_ref().map(Vec::len),
                    },
                    "domain": args.domain,
                    "answer": answer,
                    "max_reverse_depth": MAX_REVERSE_DEPTH,
                    "grounded_count": causes.iter().filter(|cause| !cause.provisional).count(),
                    "provisional_count": causes.iter().filter(|cause| cause.provisional).count(),
                    "causes": causes,
                }))
                .map_err(|error| error.to_string())?
            );
            Ok(())
        }
        Err(error) => {
            let _ = vault.flush();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "operation": "calyx readback reverse_query",
                    "domain": args.domain,
                    "answer": answer,
                    "base_rows_written": rows.base_rows,
                    "recurrence_rows_written": rows.recurrence_rows,
                    "error_code": error.code(),
                    "error": error.to_string(),
                    "remediation": error.remediation(),
                }))
                .map_err(|error| error.to_string())?
            );
            Err(error.to_string().into())
        }
    }
}

#[derive(Debug)]
struct ReadbackArgs {
    vault: PathBuf,
    domain: String,
    answer: String,
    fixture: PathBuf,
    vault_id: String,
    salt: String,
}

impl ReadbackArgs {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut vault = None;
        let mut domain = None;
        let mut answer = None;
        let mut fixture = None;
        let mut vault_id = None;
        let mut salt = None;
        let mut index = 0;
        while index < args.len() {
            let flag = args[index].as_str();
            let value = args.get(index + 1).ok_or_else(|| USAGE.to_string())?;
            match flag {
                "--vault" => vault = Some(PathBuf::from(value)),
                "--domain" => domain = Some(value.clone()),
                "--answer" => answer = Some(value.clone()),
                "--fixture" => fixture = Some(PathBuf::from(value)),
                "--vault-id" => vault_id = Some(value.clone()),
                "--salt" => salt = Some(value.clone()),
                _ => return Err(USAGE.to_string()),
            }
            index += 2;
        }
        Ok(Self {
            vault: vault.ok_or_else(|| USAGE.to_string())?,
            domain: domain.ok_or_else(|| USAGE.to_string())?,
            answer: answer.ok_or_else(|| USAGE.to_string())?,
            fixture: fixture.ok_or_else(|| USAGE.to_string())?,
            vault_id: vault_id.ok_or_else(|| USAGE.to_string())?,
            salt: salt.ok_or_else(|| USAGE.to_string())?,
        })
    }
}

#[derive(Debug, Deserialize)]
struct ReverseFixture {
    #[serde(default)]
    domain: Option<String>,
    edges: Vec<FixtureEdge>,
    #[serde(default = "default_clock_ts")]
    clock_ts: u64,
}

#[derive(Debug, Deserialize)]
struct FixtureEdge {
    from: String,
    to: String,
    #[serde(default)]
    outcome: Option<AnchorValue>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default = "default_grounded")]
    grounded: bool,
    #[serde(default = "default_occurrences")]
    occurrences: u64,
    #[serde(default)]
    structural_only: bool,
    #[serde(default)]
    confidence: Option<f32>,
    #[serde(default)]
    malformed_context: bool,
}

impl ReverseFixture {
    fn read(path: &Path, domain: &str) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|error| format!("read fixture: {error}"))?;
        let fixture: Self =
            serde_json::from_slice(&bytes).map_err(|error| format!("parse fixture: {error}"))?;
        if fixture
            .domain
            .as_deref()
            .is_some_and(|value| value != domain)
        {
            return Err("reverse_query fixture domain does not match --domain".to_string());
        }
        fixture.validate()?;
        Ok(fixture)
    }

    fn validate(&self) -> Result<(), String> {
        for edge in &self.edges {
            if edge.from.trim().is_empty() || edge.to.trim().is_empty() {
                return Err("reverse_query edge endpoints must be non-empty".to_string());
            }
            if !edge.structural_only && edge.occurrences == 0 {
                return Err(
                    "reverse_query recurrence edge occurrences must be positive".to_string()
                );
            }
            if edge
                .confidence
                .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
            {
                return Err("reverse_query confidence must be finite and in [0,1]".to_string());
            }
        }
        Ok(())
    }

    fn persist_rows(
        &self,
        vault: &AsterVault,
        vault_id: VaultId,
        domain: &str,
    ) -> Result<RowsWritten, String> {
        let mut rows = RowsWritten::default();
        for (index, edge) in self.edges.iter().enumerate() {
            if edge.structural_only {
                write_structural_edge(vault, vault_id, domain, edge, index)?;
                rows.base_rows += 1;
            } else {
                rows.base_rows += 1;
                rows.recurrence_rows +=
                    write_recurrence_edge(vault, vault_id, domain, edge, index)?;
            }
        }
        Ok(rows)
    }
}

#[derive(Debug, Default)]
struct RowsWritten {
    base_rows: usize,
    recurrence_rows: usize,
}

fn write_recurrence_edge(
    vault: &AsterVault,
    vault_id: VaultId,
    domain: &str,
    edge: &FixtureEdge,
    index: usize,
) -> Result<usize, String> {
    let series_key = format!("{index}-{}-{}", edge.from, edge.to);
    let cx_id = cx_id(domain, &edge.from, &series_key);
    write_base(
        vault,
        fixture_constellation(vault_id, cx_id, domain, &edge.from, None),
    )?;
    for occurrence_id in 0..edge.occurrences {
        let context = if edge.malformed_context {
            b"not-json".to_vec()
        } else {
            edge_context(domain, edge)?
        };
        let occurrence = Occurrence {
            id: OccurrenceId(occurrence_id),
            t_k: EpochSecs(1_000 + occurrence_id as i64),
            context: OccurrenceContext::new(context).map_err(|error| error.to_string())?,
        };
        vault
            .write_cf(
                ColumnFamily::Recurrence,
                recurrence_key(cx_id, occurrence_id),
                encode_recurrence_row(&StoredRecurrenceRow::Occurrence(occurrence))
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(edge.occurrences as usize)
}

fn write_structural_edge(
    vault: &AsterVault,
    vault_id: VaultId,
    domain: &str,
    edge: &FixtureEdge,
    index: usize,
) -> std::result::Result<(), String> {
    let series_key = format!("{index}-{}-structural", edge.from);
    let cx_id = cx_id(domain, &edge.from, &series_key);
    let confidence = edge.confidence.unwrap_or(0.35);
    write_base(
        vault,
        fixture_constellation(
            vault_id,
            cx_id,
            domain,
            &edge.from,
            Some((edge.outcome(), confidence)),
        ),
    )
}

fn write_base(vault: &AsterVault, cx: Constellation) -> std::result::Result<(), String> {
    vault
        .write_cf(
            ColumnFamily::Base,
            base_key(cx.cx_id),
            encode::encode_constellation_base(&cx).map_err(|error| error.to_string())?,
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn fixture_constellation(
    vault_id: VaultId,
    cx_id: CxId,
    domain: &str,
    action: &str,
    structural: Option<(AnchorValue, f32)>,
) -> Constellation {
    let mut metadata = BTreeMap::new();
    metadata.insert(ORACLE_DOMAIN_METADATA_KEY.to_string(), domain.to_string());
    metadata.insert(ORACLE_ACTION_METADATA_KEY.to_string(), action.to_string());
    let mut flags = CxFlags::default();
    if let Some((answer, confidence)) = structural {
        flags.ungrounded = true;
        metadata.insert(
            ORACLE_EFFECT_METADATA_KEY.to_string(),
            serde_json::to_string(&answer).expect("serialize anchor value"),
        );
        metadata.insert(
            ORACLE_STRUCTURAL_CONFIDENCE_METADATA_KEY.to_string(),
            confidence.to_string(),
        );
    }
    Constellation {
        cx_id,
        vault_id,
        panel_version: 438,
        created_at: 1,
        input_ref: InputRef {
            hash: [cx_id.as_bytes()[0]; 32],
            pointer: Some("synthetic://reverse-query".to_string()),
            redacted: true,
        },
        modality: Modality::Structured,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags,
    }
}

impl FixtureEdge {
    fn outcome(&self) -> AnchorValue {
        self.outcome
            .clone()
            .unwrap_or_else(|| AnchorValue::Text(self.to.clone()))
    }
}

fn edge_context(default_domain: &str, edge: &FixtureEdge) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({
        "action": edge.from,
        "consequences": [{
            "action_or_event": edge.to,
            "domain": edge.domain.as_deref().unwrap_or(default_domain),
            "outcome": { "value": edge.outcome() },
            "grounded": edge.grounded,
            "provisional": !edge.grounded,
        }]
    }))
    .map_err(|error| error.to_string())
}

fn read_ledger_row(vault: &AsterVault, ledger_ref: &LedgerRef) -> Result<Vec<u8>, String> {
    vault
        .read_cf_at(
            vault.latest_seq(),
            ColumnFamily::Ledger,
            &ledger_key(ledger_ref.seq),
        )
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("ledger row {} not found", ledger_ref.seq))
}

fn cx_id(domain: &str, action: &str, series_key: &str) -> CxId {
    CxId::from_bytes(content_address([
        domain.as_bytes(),
        action.as_bytes(),
        series_key.as_bytes(),
    ]))
}

fn default_clock_ts() -> u64 {
    1
}

fn default_grounded() -> bool {
    true
}

fn default_occurrences() -> u64 {
    1
}
