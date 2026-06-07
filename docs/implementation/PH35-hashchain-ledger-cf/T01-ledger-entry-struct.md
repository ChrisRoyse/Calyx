# PH35 · T01 — `LedgerEntry` struct + `EntryKind` enum + `entry_hash`

| Field | Value |
|---|---|
| **Phase** | PH35 — Hash-chain append-only CF (in group-commit) |
| **Stage** | S7 — Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/entry.rs` (≤500), `crates/calyx-ledger/src/kind.rs` (≤500) |
| **Depends on** | — (first card; reuses `LedgerRef` from `calyx-core`) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 §2` |

## Goal

Define the canonical `LedgerEntry` struct and `EntryKind` enum exactly as
specified in the PRD, and implement the deterministic `entry_hash` function
using BLAKE3. Every other ledger task depends on these types being stable and
byte-exact.

## Build (checklist of concrete, code-level steps)

- [ ] Define `EntryKind` in `kind.rs`:
  ```rust
  pub enum EntryKind {
      Ingest, Measure, Assay, Kernel, Guard, Answer,
      Anneal, Migrate, Admin, Erase,
  }
  ```
  Derive `Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize`.
  Provide `u8` wire codes (0–9 in declaration order) via `fn wire_code(self) -> u8`.
- [ ] Define `LedgerEntry` in `entry.rs` exactly:
  ```rust
  pub struct LedgerEntry {
      pub seq:        u64,
      pub prev_hash:  [u8; 32],
      pub kind:       EntryKind,
      pub subject:    SubjectId,   // CxId | LensId | KernelId | GuardId | QueryId — tagged union
      pub payload:    Vec<u8>,     // opaque zstd-compressed typed payload bytes
      pub actor:      ActorId,     // AgentId | ServiceId — tagged union
      pub ts:         u64,         // server-stamped monotonic nanoseconds UTC
      pub entry_hash: [u8; 32],
  }
  ```
- [ ] Implement `fn compute_entry_hash(seq: u64, prev_hash: &[u8; 32], kind: EntryKind, subject: &SubjectId, payload: &[u8], actor: &ActorId, ts: u64) -> [u8; 32]` using `blake3`:
  `entry_hash = blake3(seq ‖ prev_hash ‖ kind ‖ subject ‖ payload ‖ actor ‖ ts)`
  — use length-delimited framing (matching `full_content_hash` in `calyx-aster/cf/key.rs`).
- [ ] `LedgerEntry::new(...)` constructor that calls `compute_entry_hash` and sets `entry_hash`.
- [ ] `LedgerEntry::verify(&self) -> bool` — recomputes hash, returns whether `self.entry_hash` matches.
- [ ] Define `SubjectId` (tagged enum) and `ActorId` (tagged enum) in `entry.rs`.
- [ ] All types: `Clone, Debug, PartialEq, Eq, Serialize, Deserialize`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: construct a `LedgerEntry` with fixed inputs (seq=1, prev_hash=[0u8;32],
  kind=Ingest, subject=CxId([1u8;16]), payload=b"test", actor=ServiceId("svc"), ts=1_785_000_000);
  assert `entry.verify() == true`; store the 32-byte `entry_hash` as a golden constant and assert byte-exact.
- [ ] proptest: `compute_entry_hash` is deterministic: for any `(seq, prev_hash, kind, payload, ts)`,
  calling twice gives identical `[u8; 32]`.
- [ ] proptest: changing any single field (seq, prev_hash, kind, subject, payload, actor, ts)
  produces a different hash — tamper detection holds.
- [ ] edge (≥3): `seq=0` (genesis entry, `prev_hash=[0u8;32]`); `payload=&[]` (empty payload);
  `ts=u64::MAX` (max timestamp); `payload` containing UTF-8 special chars.
- [ ] fail-closed: `LedgerEntry::verify` on a struct with a corrupted `entry_hash` byte
  returns `false` (not an error — caller decides whether to quarantine).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `crates/calyx-ledger/src/entry.rs` + compiled test binary output
- **Readback:** `cargo test -p calyx-ledger -- --nocapture entry_hash_golden 2>&1 | xxd`
  — the 32-byte golden hash must match the hard-coded constant in the test.
- **Prove:** before: no `entry_hash` function exists; after: `cargo test` prints the
  32-byte hash and asserts it equals the golden constant; the hash changes when any
  input byte flips (proptest confirms); no field named `secret` or `raw_text` appears
  in the struct.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH35 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
