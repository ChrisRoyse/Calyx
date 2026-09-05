use calyx_core::{CalyxError, Clock, Result, Seq};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::cf::{
    ColumnFamily, KeyRange, WEAVE_XTERM_GENERATION_SCHEMA, WEAVE_XTERM_STORAGE_SCHEMA_DETAIL,
    weave_xterm_generation_prefix_range,
};
use crate::mvcc::tombstone_value;
use crate::vault::{AsterVault, encode};

use super::key::GraphKeyspace;
use super::{
    GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL, GRAPH_GENERATION_STORAGE_SCHEMA,
    GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL, GraphCollectionGenerationReadback,
    GraphCollectionGenerationState, GraphCollectionGenerationStatus, GraphCollectionLifecycle,
    graph_generation_storage_collection,
};

#[derive(Clone, Debug, Serialize)]
pub struct GraphGenerationReclaimCfReport {
    pub cf: String,
    pub rows_reclaimed: u64,
    pub key_bytes: u64,
    pub value_bytes: u64,
    pub source_sha256: String,
    pub visible_rows_after: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphGenerationReclaimReport {
    pub collection: String,
    pub generation: String,
    pub storage_collection: String,
    pub prior_status: GraphCollectionGenerationStatus,
    pub batch_rows: usize,
    pub graph: GraphGenerationReclaimCfReport,
    pub xterm: GraphGenerationReclaimCfReport,
    pub lifecycle_write_seq: Option<Seq>,
    pub lifecycle: GraphCollectionGenerationReadback,
    pub accepted_lifecycle_sha256: Vec<String>,
    pub idempotent: bool,
}

pub fn reclaim_graph_generation<C: Clock>(
    vault: &AsterVault<C>,
    collection: &str,
    generation: &str,
    batch_rows: usize,
) -> Result<GraphGenerationReclaimReport> {
    if !(1..=16_384).contains(&batch_rows) {
        return Err(CalyxError {
            code: "CALYX_GRAPH_GENERATION_RECLAIM_INVALID",
            message: format!("reclaim batch_rows {batch_rows} is outside 1..=16384"),
            remediation: "choose a bounded reclaim batch size in 1..=16384",
        });
    }
    vault.with_durable_commit_lock(|| {
        reclaim_graph_generation_locked(vault, collection, generation, batch_rows)
    })
}

fn reclaim_graph_generation_locked<C: Clock>(
    vault: &AsterVault<C>,
    collection: &str,
    generation: &str,
    batch_rows: usize,
) -> Result<GraphGenerationReclaimReport> {
    let lifecycle = GraphCollectionLifecycle::new(vault)?;
    let before = lifecycle.list_states(vault.latest_seq())?;
    let target = before
        .iter()
        .find(|row| row.state.collection == collection && row.state.generation == generation)
        .cloned()
        .ok_or_else(|| CalyxError {
            code: "CALYX_GRAPH_GENERATION_NOT_FOUND",
            message: format!("graph generation {collection}/{generation} has no lifecycle row"),
            remediation: "list graph-collection-generations and pass an exact failed or tombstoned generation",
        })?;
    match &target.state.status {
        GraphCollectionGenerationStatus::Failed | GraphCollectionGenerationStatus::Tombstoned => {}
        GraphCollectionGenerationStatus::Accepted => {
            return Err(reclaim_refused(
                collection,
                generation,
                "accepted",
                "accepted generations are immutable serving state",
            ));
        }
        GraphCollectionGenerationStatus::Writing => {
            return Err(reclaim_refused(
                collection,
                generation,
                "writing",
                "a writing generation may still have a live producer",
            ));
        }
    }

    let storage_collection = graph_generation_storage_collection(collection, generation)?;
    require_detail(
        &target.state,
        GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL,
        GRAPH_GENERATION_STORAGE_SCHEMA,
    )?;
    require_detail(
        &target.state,
        GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL,
        &storage_collection,
    )?;
    require_detail(
        &target.state,
        WEAVE_XTERM_STORAGE_SCHEMA_DETAIL,
        WEAVE_XTERM_GENERATION_SCHEMA,
    )?;

    let accepted_before = accepted_lifecycle_sha256(&before, collection);
    let graph_range = GraphKeyspace::new(&storage_collection)?.collection_range();
    let xterm_range = weave_xterm_generation_prefix_range(collection, generation)?;
    let mut graph = reclaim_range_locked(vault, ColumnFamily::Graph, &graph_range, batch_rows)?;
    let mut xterm = reclaim_range_locked(vault, ColumnFamily::XTerm, &xterm_range, batch_rows)?;

    let rows_reclaimed = graph.rows_reclaimed + xterm.rows_reclaimed;
    if rows_reclaimed > 0 {
        vault.refresh_reconstructible_derived_cfs_locked([
            ColumnFamily::Graph,
            ColumnFamily::XTerm,
        ])?;
    }
    graph.visible_rows_after = visible_rows(vault, ColumnFamily::Graph, &graph_range)?;
    xterm.visible_rows_after = visible_rows(vault, ColumnFamily::XTerm, &xterm_range)?;
    if graph.visible_rows_after != 0 || xterm.visible_rows_after != 0 {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "graph generation {collection}/{generation} remains visible after reclaim: graph={} xterm={}",
            graph.visible_rows_after, xterm.visible_rows_after
        )));
    }

    if rows_reclaimed > 0 {
        vault.purge_tombstoned_cfs_locked(&[ColumnFamily::Graph, ColumnFamily::XTerm])?;
    }

    let idempotent =
        target.state.status == GraphCollectionGenerationStatus::Tombstoned && rows_reclaimed == 0;
    let lifecycle_write_seq = if idempotent {
        None
    } else {
        let mut tombstoned = GraphCollectionGenerationState::new(
            collection,
            generation,
            GraphCollectionGenerationStatus::Tombstoned,
            "graph-generation-reclaim",
        )
        .with_reason("failed isolated Graph/XTerm payload was physically reclaimed and cold-zero verification is required before closeout");
        tombstoned.detail = target.state.detail.clone();
        tombstoned.detail.insert(
            "reclaimed_graph_rows".to_string(),
            graph.rows_reclaimed.to_string(),
        );
        tombstoned.detail.insert(
            "reclaimed_xterm_rows".to_string(),
            xterm.rows_reclaimed.to_string(),
        );
        tombstoned.detail.insert(
            "reclaim_source_sha256_graph".to_string(),
            graph.source_sha256.clone(),
        );
        tombstoned.detail.insert(
            "reclaim_source_sha256_xterm".to_string(),
            xterm.source_sha256.clone(),
        );
        let seq = lifecycle.put_state_locked(&tombstoned)?;
        vault.flush_locked()?;
        Some(seq)
    };

    let after = lifecycle.list_states(vault.latest_seq())?;
    let accepted_after = accepted_lifecycle_sha256(&after, collection);
    if accepted_before != accepted_after {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "accepted lifecycle state changed while reclaiming {collection}/{generation}"
        )));
    }
    let lifecycle = after
        .into_iter()
        .find(|row| row.state.collection == collection && row.state.generation == generation)
        .ok_or_else(|| {
            CalyxError::aster_corrupt_shard(format!(
                "reclaimed lifecycle row disappeared for {collection}/{generation}"
            ))
        })?;
    if lifecycle.state.status != GraphCollectionGenerationStatus::Tombstoned {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "reclaimed lifecycle row is {}, expected tombstoned",
            lifecycle.state.status.as_str()
        )));
    }

    Ok(GraphGenerationReclaimReport {
        collection: collection.to_string(),
        generation: generation.to_string(),
        storage_collection,
        prior_status: target.state.status.clone(),
        batch_rows,
        graph,
        xterm,
        lifecycle_write_seq,
        lifecycle,
        accepted_lifecycle_sha256: accepted_after,
        idempotent,
    })
}

fn reclaim_range_locked<C: Clock>(
    vault: &AsterVault<C>,
    cf: ColumnFamily,
    range: &KeyRange,
    batch_rows: usize,
) -> Result<GraphGenerationReclaimCfReport> {
    let mut after_key = None::<Vec<u8>>;
    let mut rows_reclaimed = 0u64;
    let mut key_bytes = 0u64;
    let mut value_bytes = 0u64;
    let mut digest = Sha256::new();
    loop {
        // The outer durable file lock excludes every other Aster writer for
        // the entire reclaim. Read each page at the handle's current tip,
        // then advance `after_key` before committing its tombstones. This is
        // the only bounded paging model valid for write_latest_readback:
        // after the first reconstructible commit, that profile deliberately
        // cannot serve the now-historical opening sequence. No target-prefix
        // row can appear or change between pages because failed/tombstoned
        // lifecycle state excludes a producer and the cross-process writer
        // lock remains held.
        let page = vault.scan_cf_range_page_at(
            vault.latest_seq(),
            cf,
            range,
            after_key.as_deref(),
            batch_rows,
        )?;
        let Some((last_key, _)) = page.last() else {
            break;
        };
        after_key = Some(last_key.clone());
        let tombstone = tombstone_value();
        let mut rows = Vec::with_capacity(page.len());
        for (key, value) in page {
            rows_reclaimed = rows_reclaimed.saturating_add(1);
            key_bytes = key_bytes.saturating_add(key.len() as u64);
            value_bytes = value_bytes.saturating_add(value.len() as u64);
            digest.update((key.len() as u64).to_be_bytes());
            digest.update(&key);
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(&value);
            rows.push(encode::WriteRow {
                cf,
                key,
                value: tombstone.clone(),
            });
        }
        vault.commit_reconstructible_rows_locked(&rows)?;
    }
    Ok(GraphGenerationReclaimCfReport {
        cf: cf.name().to_string(),
        rows_reclaimed,
        key_bytes,
        value_bytes,
        source_sha256: format!("{:x}", digest.finalize()),
        visible_rows_after: u64::MAX,
    })
}

fn visible_rows<C: Clock>(
    vault: &AsterVault<C>,
    cf: ColumnFamily,
    range: &KeyRange,
) -> Result<u64> {
    let page = vault.scan_cf_range_page_at(vault.latest_seq(), cf, range, None, 1)?;
    Ok(page.len() as u64)
}

fn accepted_lifecycle_sha256(
    rows: &[GraphCollectionGenerationReadback],
    collection: &str,
) -> Vec<String> {
    let mut values = rows
        .iter()
        .filter(|row| {
            row.state.collection == collection
                && row.state.status == GraphCollectionGenerationStatus::Accepted
        })
        .map(|row| format!("{}:{}", row.state.generation, row.value_sha256))
        .collect::<Vec<_>>();
    values.sort();
    values
}

fn require_detail(
    state: &GraphCollectionGenerationState,
    key: &'static str,
    expected: &str,
) -> Result<()> {
    match state.detail.get(key) {
        Some(actual) if actual == expected => Ok(()),
        actual => Err(CalyxError::aster_corrupt_shard(format!(
            "graph generation {}/{} detail {key} is {:?}, expected {expected}",
            state.collection, state.generation, actual
        ))),
    }
}

fn reclaim_refused(collection: &str, generation: &str, status: &str, reason: &str) -> CalyxError {
    CalyxError {
        code: "CALYX_GRAPH_GENERATION_RECLAIM_REFUSED",
        message: format!(
            "refusing to reclaim graph generation {collection}/{generation} in {status} state: {reason}"
        ),
        remediation: "reclaim only an exact failed or already tombstoned isolated generation; never override accepted or writing state",
    }
}
