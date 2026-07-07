use crate::raw_sources::RawSourceInventory;

pub(crate) fn inventory_readme(inventory: &RawSourceInventory) -> String {
    format!(
        "# Polymarket raw source inventory\n\nsource_of_truth: {}\nstatus_code: {}\npassed: {}\nsamples: {}\nrequired_failures: {}\nedge_cases: {}\nbytes: {}\ndocs_index_status: {}\ndocs_index_rows: {}\ndocs_index_not_yet_sampled: {}\ndocs_index_blocked_runtime: {}\n\nSchema note: raw payloads are preserved before database modeling. Gamma currently exposes several arrays as JSON-encoded strings, CLOB/Data API expose related identifiers with different field names, and the public market WebSocket emits frame/event streams that must be modeled separately from HTTP snapshots.\n",
        inventory.source_of_truth,
        inventory.status_code,
        inventory.passed,
        inventory.coverage.sample_count,
        inventory.coverage.required_failure_count,
        inventory.coverage.edge_case_count,
        inventory.coverage.total_body_bytes,
        inventory.docs_index_coverage.status_code,
        inventory.docs_index_coverage.row_count,
        inventory.docs_index_coverage.not_yet_sampled_count,
        inventory.docs_index_coverage.blocked_runtime_count
    )
}
