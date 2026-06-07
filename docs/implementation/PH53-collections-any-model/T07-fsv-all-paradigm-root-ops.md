# PH53 · T07 — FSV: each paradigm's root op round-trips by readback on aiwonder

| Field | Value |
|---|---|
| **Phase** | PH53 — Collections-as-any-model (relational/doc/KV/TS/blob) |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/tests/ph53_fsv.rs` (≤500) |
| **Depends on** | T02, T03, T04, T05, T06 |
| **Axioms** | A15, A16, A19 |
| **PRD** | `dbprdplans/20 §2/§3`, `dbprdplans/04 §2` |

## Goal

A single, seeded integration test that exercises every paradigm's root
operation end-to-end: `point` (KV), `range` (relational + TS), `join-by-ref`
(relational), `aggregate` (TS rollup), `traverse` (document subtree), and
`rollup` (TS). The test opens a real vault, writes data through each layer,
restarts the vault (drop + reopen), reads back bytes, and asserts byte-exact
equality. This is the phase FSV gate: no harness assertion counts — the bytes
on disk are the truth.

## Build (checklist of concrete, code-level steps)

- [ ] Create `tests/ph53_fsv.rs` with a `#[test] fn ph53_all_paradigm_roundtrips()`.
- [ ] Use a fixed `TempDir` with a deterministic seed path
  (`/tmp/calyx-ph53-fsv-test`); clean it at test start.
- [ ] **Relational (point + range + join-by-ref):**
  - Create `orders` (`Records`, `SchemaFull`) and `products` (`Records`, `SchemaFull`).
  - `put_record(orders, pk=1, {"product_id":42,"qty":3})`.
  - `put_record(products, pk=42, {"name":"bolt","price_cents":100})`.
  - `get_record(orders, pk=1)` → assert `qty==3`.
  - `range(orders, 0, 100)` → assert 1 row.
  - `join_by_ref(orders, pk=1, "products", "product_id")` → assert `name=="bolt"`.
- [ ] **Document (traverse/subtree):**
  - Create `docs` (`Documents`, `SchemaLess`).
  - `put_doc(id="d1", {"meta":{"author":"alice"},"body":"hello"})`.
  - `get_subtree("d1", ["meta"])` → assert `author=="alice"`; body not present.
- [ ] **KV (point):**
  - Create `cache` (`KV`, `SchemaLess`).
  - `kv_set(ns=1, key=b"x", val=b"42", ttl=None)`.
  - `kv_get(ns=1, key=b"x")` → assert `Some(b"42")`.
- [ ] **Time-series (range + rollup):**
  - Create `metrics` (`TimeSeries`, `SchemaLess`).
  - Write 3 points with fixed `ts` values (seeded).
  - `ts_range(0, u64::MAX)` → assert 3 points in asc order.
  - `ts_rollup(OneHour)` → assert `count=3`, exact `sum`.
- [ ] **Blob (point):**
  - Create `assets` (`Blob`, `SchemaLess`).
  - `blob_put(id=b1, data=b"calyx-ph53-blob-test" * 512)`.  // ~10 KiB
  - `blob_get(b1)` → assert byte-exact.
- [ ] **Vault restart:** drop the vault handle; reopen from same path; re-run all
  `get_*` assertions → all must pass byte-exact (proves WAL/SST durability).
- [ ] **0-lens check:** assert none of the above collections wrote to `slot_00` CF.
- [ ] All assertions use `assert_eq!` with exact expected bytes/values (no fuzzy
  match, no harness-as-FSV).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] The test itself is the FSV proof; it must be 100% deterministic (seed RNG
  with `42`, pin clock with injected `Clock`).
- [ ] proptest: not required here — covered in T02–T06; this card is the
  integration gate.
- [ ] edge: vault restart mid-run (T06 already covers crash; here we do clean
  restart only).
- [ ] fail-closed: all `Result` returns checked; first `Err` fails the test with
  the `CALYX_*` code printed.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cf/relational/`, `cf/document/`, `cf/kv/`, `cf/timeseries/`,
  `cf/blob/` SST shards after vault restart.
- **Readback:**
  ```
  cargo test -p calyx-aster ph53_all_paradigm_roundtrips -- --nocapture 2>&1 | tail -20
  xxd /tmp/calyx-ph53-fsv-test/cf/relational/000001.sst | head -4
  xxd /tmp/calyx-ph53-fsv-test/cf/document/000001.sst  | head -4
  xxd /tmp/calyx-ph53-fsv-test/cf/kv/000001.sst        | head -4
  xxd /tmp/calyx-ph53-fsv-test/cf/timeseries/000001.sst| head -4
  xxd /tmp/calyx-ph53-fsv-test/cf/blob/000001.sst      | head -4
  ```
- **Prove:** Test prints "ph53 FSV: all paradigm round-trips PASS" and exits 0;
  each `xxd` shows the correct discriminant byte (`0x01`–`0x05`) at the start of
  the first key; `slot_00` CF does not exist or is 0 bytes.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (test output + `xxd` screenshots) attached to the PH53 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
