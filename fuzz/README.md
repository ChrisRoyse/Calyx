# Calyx Fuzz Targets

Run in a manual verification run from the repo root:

```bash
cargo fuzz list
cargo fuzz run aster_sst_decode fuzz/corpus/aster_sst_decode -- -runs=1000
cargo fuzz run aster_wal_replay fuzz/corpus/aster_wal_replay -- -runs=1000
cargo fuzz run aster_manifest_decode fuzz/corpus/aster_manifest_decode -- -runs=1000
cargo fuzz run query_parse fuzz/corpus/query_parse -- -runs=1000
cargo fuzz run lens_output_decode fuzz/corpus/lens_output_decode -- -runs=1000
cargo fuzz run mcp_jsonrpc_decode fuzz/corpus/mcp_jsonrpc_decode -- -runs=1000
```

Targets map to PRD `28 §6c` untrusted-input boundaries:

- `aster_sst_decode`: Aster SST/shard reader via `SstReader::open`.
- `aster_wal_replay`: WAL segment replay via `wal::replay_dir`.
- `aster_manifest_decode`: durable vault manifest load via `ManifestStore`.
- `query_parse`: Sextant query JSON decode, validation, and planning bounds.
- `lens_output_decode`: SlotVector JSON/raw f32 output schema validation for dense, sparse, and multi-vector payloads.
- `mcp_jsonrpc_decode`: MCP JSON-RPC request/batch wire decode.

Seed corpora should include real persisted bytes copied from manual evidence vaults for Aster SST/WAL/MANIFEST cases. Fuzzer artifacts are evidence until triaged: every crash input gets a GitHub issue plus a regression test.
