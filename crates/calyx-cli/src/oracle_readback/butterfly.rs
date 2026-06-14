use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

use calyx_aster::cf::{ColumnFamily, base_key, recurrence_key};
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::{
    Occurrence, OccurrenceContext, StoredRecurrenceRow, encode_recurrence_row,
};
use calyx_aster::vault::encode;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AnchorValue, Clock, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality,
    VaultId, content_address,
};
use calyx_oracle::{
    Consequence, ConsequenceTree, DomainId, HOP_ATTENUATION, MAX_DEPTH, MIN_CONFIDENCE_THRESHOLD,
    ORACLE_ACTION_METADATA_KEY, ORACLE_DOMAIN_METADATA_KEY, build_tree, is_provisional_ledger_ref,
    select,
};
use serde::Deserialize;
use serde_json::json;

pub(crate) fn readback_oracle_expand(args: &[String]) -> crate::error::CliResult {
    let args = ExpandArgs::parse(args)?;
    let vault_id = VaultId::from_str(&args.vault_id)
        .map_err(|error| format!("invalid --vault-id: {error}"))?;
    let vault = AsterVault::new_durable(
        Path::new(&args.vault),
        vault_id,
        args.salt.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let fixture = ButterflyFixture::read(Path::new(&args.fixture))?;
    let rows_written = fixture.persist_rows(&vault)?;
    vault
        .flush()
        .map_err(|error| format!("flush oracle expand fixture rows: {error}"))?;
    let clock = FixedClock::new(fixture.clock_ts);
    match build_tree(&vault, fixture.root(), &clock) {
        Ok(tree) => {
            let tree = prune_tree(tree, args.depth);
            let flat = flatten_descendants(&tree);
            let selected = fixture
                .desired_outcome
                .as_ref()
                .and_then(|desired| select(&tree, desired))
                .map(|node| node.root.clone());
            vault
                .flush()
                .map_err(|error| format!("flush oracle expand ledger row: {error}"))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "operation": "calyx readback oracle_expand",
                    "domain": fixture.domain,
                    "rows_written": rows_written,
                    "requested_depth": args.depth,
                    "max_depth": MAX_DEPTH,
                    "hop_attenuation": HOP_ATTENUATION,
                    "min_confidence_threshold": MIN_CONFIDENCE_THRESHOLD,
                    "max_observed_hop": max_observed_hop(&tree),
                    "provisional_count": provisional_count(&tree),
                    "flat": flat,
                    "selected": selected,
                    "tree": tree,
                }))
                .map_err(|error| error.to_string())?
            );
            Ok(())
        }
        Err(error) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "operation": "calyx readback oracle_expand",
                    "domain": fixture.domain,
                    "rows_written": rows_written,
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

struct ExpandArgs {
    vault: String,
    fixture: String,
    vault_id: String,
    salt: String,
    depth: u8,
}

impl ExpandArgs {
    fn parse(args: &[String]) -> Result<Self, String> {
        match args {
            [vault_flag, vault, fixture_flag, fixture, vault_id_flag, vault_id, salt_flag, salt]
                if vault_flag == "--vault"
                    && fixture_flag == "--fixture"
                    && vault_id_flag == "--vault-id"
                    && salt_flag == "--salt" =>
            {
                Ok(Self {
                    vault: vault.clone(),
                    fixture: fixture.clone(),
                    vault_id: vault_id.clone(),
                    salt: salt.clone(),
                    depth: MAX_DEPTH,
                })
            }
            [
                vault_flag,
                vault,
                fixture_flag,
                fixture,
                vault_id_flag,
                vault_id,
                salt_flag,
                salt,
                depth_flag,
                depth,
            ] if vault_flag == "--vault"
                && fixture_flag == "--fixture"
                && vault_id_flag == "--vault-id"
                && salt_flag == "--salt"
                && depth_flag == "--depth" =>
            {
                let depth = depth
                    .parse::<u8>()
                    .map_err(|error| format!("invalid --depth: {error}"))?;
                if depth > MAX_DEPTH {
                    return Err(format!("oracle_expand --depth must be <= {MAX_DEPTH}"));
                }
                Ok(Self {
                    vault: vault.clone(),
                    fixture: fixture.clone(),
                    vault_id: vault_id.clone(),
                    salt: salt.clone(),
                    depth,
                })
            }
            _ => Err("usage: calyx readback oracle_expand --vault <dir> --fixture <json> --vault-id <id> --salt <s> [--depth <0-4>]".to_string()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ButterflyFixture {
    domain: String,
    root_action: String,
    root_outcome: AnchorValue,
    #[serde(default = "default_root_confidence")]
    root_confidence: f32,
    #[serde(default)]
    root_hop: u8,
    edges: Vec<FixtureEdge>,
    #[serde(default)]
    desired_outcome: Option<AnchorValue>,
    #[serde(default = "default_clock_ts")]
    clock_ts: u64,
}

#[derive(Debug, Deserialize)]
struct FixtureEdge {
    from: String,
    to: String,
    outcome: AnchorValue,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default = "default_grounded")]
    grounded: bool,
    #[serde(default)]
    malformed_context: bool,
}

impl ButterflyFixture {
    fn read(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|error| format!("read fixture: {error}"))?;
        let fixture: Self =
            serde_json::from_slice(&bytes).map_err(|error| format!("parse fixture: {error}"))?;
        fixture.validate()?;
        Ok(fixture)
    }

    fn validate(&self) -> Result<(), String> {
        if self.domain.trim().is_empty() {
            return Err("oracle_expand fixture domain must be non-empty".to_string());
        }
        if self.root_action.trim().is_empty() {
            return Err("oracle_expand fixture root_action must be non-empty".to_string());
        }
        if !self.root_confidence.is_finite() || !(0.0..=1.0).contains(&self.root_confidence) {
            return Err("oracle_expand fixture root_confidence must be in [0,1]".to_string());
        }
        if self.root_hop > MAX_DEPTH {
            return Err(format!(
                "oracle_expand fixture root_hop must be <= {MAX_DEPTH}"
            ));
        }
        for edge in &self.edges {
            if edge.from.trim().is_empty() || edge.to.trim().is_empty() {
                return Err("oracle_expand fixture edge endpoints must be non-empty".to_string());
            }
        }
        Ok(())
    }

    fn root(&self) -> Consequence {
        Consequence {
            action_or_event: self.root_action.clone(),
            domain: DomainId::from(self.domain.clone()),
            outcome: self.root_outcome.clone(),
            confidence: self.root_confidence,
            hop: self.root_hop,
            provenance: LedgerRef {
                seq: 0,
                hash: [0; 32],
            },
        }
    }

    fn persist_rows<C>(&self, vault: &AsterVault<C>) -> Result<usize, String>
    where
        C: Clock,
    {
        let mut rows = 0;
        for (index, edge) in self.edges.iter().enumerate() {
            rows += write_edge(vault, &self.domain, edge, index)?;
        }
        Ok(rows)
    }
}

fn write_edge<C>(
    vault: &AsterVault<C>,
    domain: &str,
    edge: &FixtureEdge,
    index: usize,
) -> Result<usize, String>
where
    C: Clock,
{
    let series_key = format!("{index}-{}-{}", edge.from, edge.to);
    let cx_id = cx_id(domain, &edge.from, &series_key);
    vault
        .write_cf(
            ColumnFamily::Base,
            base_key(cx_id),
            encode::encode_constellation_base(&fixture_constellation(
                vault.vault_id(),
                cx_id,
                domain,
                &edge.from,
            ))
            .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    let occurrence = Occurrence {
        id: OccurrenceId(0),
        t_k: EpochSecs(1_000 + index as i64),
        context: OccurrenceContext::new(edge_context_bytes(domain, edge))
            .map_err(|error| error.to_string())?,
    };
    vault
        .write_cf(
            ColumnFamily::Recurrence,
            recurrence_key(cx_id, 0),
            encode_recurrence_row(&StoredRecurrenceRow::Occurrence(occurrence))
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    Ok(2)
}

fn fixture_constellation(
    vault_id: VaultId,
    cx_id: CxId,
    domain: &str,
    action: &str,
) -> Constellation {
    let mut metadata = BTreeMap::new();
    metadata.insert(ORACLE_DOMAIN_METADATA_KEY.to_string(), domain.to_string());
    metadata.insert(ORACLE_ACTION_METADATA_KEY.to_string(), action.to_string());
    Constellation {
        cx_id,
        vault_id,
        panel_version: 433,
        created_at: 1,
        input_ref: InputRef {
            hash: [cx_id.as_bytes()[0]; 32],
            pointer: None,
            redacted: false,
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
        flags: CxFlags::default(),
    }
}

fn edge_context_bytes(default_domain: &str, edge: &FixtureEdge) -> Vec<u8> {
    if edge.malformed_context {
        return b"not-json".to_vec();
    }
    edge_context(default_domain, edge)
}

fn edge_context(default_domain: &str, edge: &FixtureEdge) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "action": &edge.from,
        "consequences": [{
            "action_or_event": &edge.to,
            "domain": edge.domain.as_deref().unwrap_or(default_domain),
            "outcome": { "value": &edge.outcome },
            "grounded": edge.grounded
        }]
    }))
    .expect("fixture context json")
}

fn prune_tree(mut tree: ConsequenceTree, depth: u8) -> ConsequenceTree {
    if tree.root.hop >= depth {
        tree.children.clear();
        return tree;
    }
    tree.children = tree
        .children
        .into_iter()
        .filter(|child| child.root.hop <= depth)
        .map(|child| prune_tree(child, depth))
        .collect();
    tree
}

fn flatten_descendants(tree: &ConsequenceTree) -> Vec<Consequence> {
    let mut out = Vec::new();
    for child in &tree.children {
        out.push(child.root.clone());
        out.extend(flatten_descendants(child));
    }
    out
}

fn max_observed_hop(tree: &ConsequenceTree) -> u8 {
    tree.children
        .iter()
        .map(max_observed_hop)
        .max()
        .map_or(tree.root.hop, |child| child.max(tree.root.hop))
}

fn provisional_count(tree: &ConsequenceTree) -> usize {
    let child_count = tree
        .children
        .iter()
        .filter(|child| is_provisional_ledger_ref(&child.root.provenance))
        .count();
    child_count + tree.children.iter().map(provisional_count).sum::<usize>()
}

fn cx_id(domain: &str, action: &str, series_key: &str) -> CxId {
    CxId::from_bytes(content_address([
        domain.as_bytes(),
        action.as_bytes(),
        series_key.as_bytes(),
    ]))
}

fn default_root_confidence() -> f32 {
    1.0
}

fn default_clock_ts() -> u64 {
    1
}

fn default_grounded() -> bool {
    true
}
