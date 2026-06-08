# Stage 7 — Ledger Provenance (PH35–PH36)

Make every signal traceable input→lens→vector→cross-term→signal→answer, and
tamper-evident. Provenance is a hard requirement, in the group-commit path so it
can't be lost on crash. Lands in `calyx-ledger`. **Living-system role:** self-
knowledge / conscience. *Threads through Stage 1+ — start the chained CF as soon
as PH09 writes constellations.*

---

## PH35 — Hash-chain append-only CF (in group-commit)
- **Objective.** An append-only, hash-chained `ledger` CF; every mutation writes
  a chained entry as part of the same group-commit as the data it describes.
- **Deps.** PH09 (write path), PH05 (WAL group-commit).
- **Deliverables.** `entry.rs` (`LedgerEntry { seq, prev_hash, kind, subject,
  payload, actor, ts, entry_hash }`), append-only enforcement (no
  update/delete; tombstones forbidden on this CF), redaction (hashes/ids, not
  secret values).
- **Key tasks.** wire into PH09's group-commit; `kind ∈ {Ingest,Measure,Assay,
  Kernel,Guard,Answer,Anneal,Migrate,Admin,Erase}`; actor-stamped; server-
  stamped monotonic ts.
- **Post-sweep note.** PH35 T01 (#242) is implemented in `calyx-ledger`:
  stable `EntryKind` wire codes, `LedgerEntry`, `SubjectId`, `ActorId`,
  length-delimited BLAKE3 `entry_hash`, and golden/tamper readbacks are
  FSV-backed at `/home/croyse/calyx/data/fsv-issue242-ledger-entry-20260608`.
- **Post-sweep note.** PH35 T02 (#243) adds deterministic binary
  `encode`/`decode`/`decode_header` and `CALYX_LEDGER_CORRUPT`; golden `xxd`,
  fail-closed decode, and round-trip proptest readbacks are FSV-backed at
  `/home/croyse/calyx/data/fsv-issue243-ledger-codec-20260608`.
- **FSV gate.** every constellation write has a chained ledger entry in the WAL
  group-commit (read the WAL + ledger CF); chain links verify; no entry stores a
  secret value.
- **Axioms/PRD.** A15, `11 §1/§2`, `04 §5`.

## PH36 — Merkle checkpoints + verify_chain + reproduce()
- **Objective.** Periodic Merkle roots (signed for export) + tamper detection +
  replay of a claim.
- **Deps.** PH35.
- **Deliverables.** `merkle.rs` (range roots, Ed25519 sign for export),
  `verify_chain(range)`, `reproduce(answer_id)` (re-measure with recorded
  lens/weights, re-run recorded fusion, re-assert within tolerance).
- **Key tasks.** checkpoint cadence; `CALYX_LEDGER_CHAIN_BROKEN` quarantines the
  range (fail-closed); reproduce uses content-addressed frozen lenses +
  determinism mode (Forge).
- **FSV gate.** flip one ledger byte → `verify_chain` detects the break **at the
  right seq**; `reproduce(answer)` on a real answer is **bit-parity within
  tolerance** (read both).
- **Axioms/PRD.** A15, A16, `11 §2/§3/§5`.

---

## Stage 7 exit
Calyx is auditable to the byte — every vector/bit/kernel/guard/answer traces to
its grounded source and replays to prove it was measured, not made up — PRD
`PROVENANCE`. Every "trusted" surface elsewhere must be backed by a Ledger entry
or it is tagged `unprovenanced`.
