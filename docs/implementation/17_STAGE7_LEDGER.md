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
- **Post-sweep note.** PH35 T03 (#244) adds `LedgerAppender`, recovered
  monotonic seq, hash-chain append, and append-only delete/tombstone rejection;
  disk row readbacks are FSV-backed at
  `/home/croyse/calyx/data/fsv-issue244-ledger-appender-20260608`.
- **Post-sweep note.** PH35 T04 (#245) adds `RedactionPolicy`,
  `PayloadBuilder`, `RedactedInput`, `CALYX_LEDGER_SECRET_IN_PAYLOAD`, and
  appender-side fail-closed payload scanning before row encoding. Disk row
  payload readbacks and forbidden-string scans are FSV-backed at
  `/home/croyse/calyx/data/fsv-issue245-ledger-redaction-20260608`.
- **Post-sweep note.** PH35 T05 (#246) adds the group-commit hook that stages
  a real `LedgerEntry` row before the base/slot data rows in the same Aster WAL
  batch. WAL, ledger-CF, and SST byte readbacks are FSV-backed at
  `/home/croyse/calyx/data/fsv-issue246-ledger-group-commit-20260608`.
- **Post-sweep note.** PH35 hardening #345 changes the hook to prepare the
  ledger bytes without advancing the appender, adds them to the storage batch,
  and commits the appender tip only after the Aster batch commit succeeds.
  Failure-injected aiwonder readbacks prove no leaked row, no `next_seq`
  advance, and no visible Ledger CF row at
  `/home/croyse/calyx/data/fsv-issue345-ledger-group-commit-atomicity-20260609`.
- **Post-sweep note.** PH35 public-surface hardening #652 removes
  `LedgerGroupCommitHook` from the public re-export surface and makes direct
  `on_commit` misuse fail closed with no batch row, no durable row, and no
  appender-tip movement. Aster continues to use the staged path:
  `stage_with_checkpoints` before durable commit, `commit_staged` after durable
  commit succeeds. Direct-reject and Aster staged-commit aiwonder readbacks are
  FSV-backed at
  `/home/croyse/calyx/data/fsv-issue652-ledger-hook-surface-20260611T070209Z`.
- **Post-sweep note.** PH35 T06 (#247) adds actor validation plus
  server-stamped monotonic timestamps in `LedgerAppender`, including restart
  recovery of `last_ts` and Aster ingest readback of non-empty service actors.
  Ledger-CF, WAL, SST byte, and compact `jq` row readbacks are FSV-backed at
  `/home/croyse/calyx/data/fsv-issue247-ledger-actor-ts-20260608`.
- **Post-sweep note.** PH35 T07 (#248) adds the PH09-to-ledger integration
  smoke: 100 unique `AsterVault::put` constellation writes, 100 chained ledger
  CF rows, 100 WAL records with ledger/base co-location, ledger-before-base
  ordering, and an empty ledger secret scan. Ledger-CF, WAL, SST byte, JSON,
  and grep readbacks are FSV-backed at
  `/home/croyse/calyx/data/fsv-issue248-ledger-integration-smoke-20260608`.
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
- **Post-sweep note.** PH36 T01 (#249) adds `calyx-ledger::merkle` range roots,
  domain-separated BLAKE3 leaves/nodes, Ed25519 signed export bundles, and the
  `calyx merkle-root` CLI path. Synthetic ledger-CF rows, CLI root equivalence,
  signature round-trip/tamper, and missing-row fail-closed readbacks are
  FSV-backed at
  `/home/croyse/calyx/data/fsv-issue249-merkle-root-ed25519-20260609`.
- **Post-sweep note.** PH36 hardening #347 binds `range_start`/`range_end` into
  Merkle export signatures, preventing wrong-range replay. PH36 hardening #348
  makes `calyx merkle-root --vault` read real Aster `cf/ledger` SST rows plus
  WAL batches, fail closed for non-Aster directories, and avoid side
  `ledger`/`ledger-cf` directories. Aiwonder FSV is backed at
  `/home/croyse/calyx/data/fsv-issue347-merkle-range-bound-signatures-20260609`
  and
  `/home/croyse/calyx/data/fsv-issue348-merkle-vault-real-aster-cf-20260609`.
- **Post-sweep note.** PH36 T02 (#250) adds `verify_chain(range)`, exact
  `CALYX_LEDGER_CHAIN_BROKEN at seq=<n>` CLI reporting, and fail-closed Aster
  manifest quarantine records. Aiwonder FSV flipped seq 7 in the physical
  Ledger SSTs, wrote a manifest quarantine for `0..20`, and proved a seq 8
  read fails closed at
  `/home/croyse/calyx/data/fsv-issue250-verify-chain-quarantine-20260609`.
- **Post-sweep hardening.** #651 extends `verify_chain(range)` so missing,
  truncated, or key/encoded-sequence-mismatched physical Ledger rows return a
  structured `VerifyResult::Corrupt` with the failing seq. The CLI reports
  `CALYX_LEDGER_CORRUPT at seq=<n>` and writes the same Aster manifest
  quarantine record as hash-chain `Broken` results. Durable aiwonder evidence:
  `/home/croyse/calyx/data/fsv-issue651-verify-chain-physical-20260610`.
- **Post-sweep note.** PH36 T03 (#251) adds `checkpoint.rs`,
  `CheckpointScheduler`, `CheckpointPayload`, Aster `VaultOptions`
  checkpoint cadence, and `calyx scan --cf ledger --vault` decoded readback.
  Aiwonder FSV wrote three signed `kind=Admin` `checkpoint_v1` rows at seq
  3, 7, and 11; each payload root matched an independent
  `calyx merkle-root --vault` read over its range, and WAL readback proved
  the checkpoint rows were in the same group-commit batch as the triggering
  ingest rows at
  `/home/croyse/calyx/data/fsv-issue251-checkpoint-scheduler-20260609`.
- **Post-sweep note.** PH36 T04 (#252) adds content-addressed reproduce
  re-measure plumbing: recorded lens identity, input-hash validation, Forge
  determinism activation, and fail-closed mismatch handling. Aiwonder FSV is
  backed at
  `/home/croyse/calyx/data/fsv-issue252-reproduce-20260609`.
- **Post-sweep note.** PH36 T05 (#253) adds fusion replay/drift assertion and
  `ReproduceResult` assembly. Aiwonder FSV wrote a `reproduce_v1` ledger row
  with `reproduced=true`, `max_drift=0.0`, and intact chain readback at
  `/home/croyse/calyx/data/fsv-issue253-reproduce-fusion-20260609`.
- **Post-sweep note.** PH36 T06 (#254) adds quarantine-aware
  `get_provenance`, `get_answer_trace`, and `audit(filter)` surfaces. Aiwonder
  FSV proved complete Kernel/Guard/Answer traces, unprovenanced partial traces,
  and quarantined range fail-closed behavior at
  `/home/croyse/calyx/data/fsv-issue254-audit-query-20260609`.
- **Post-sweep note.** PH36 T07 (#255) integrates flip-byte tamper detection
  and reproduce bit-parity in one PH36 exit bundle. Aiwonder FSV flipped ledger
  seq 11, wrote manifest quarantine `0..20`, denied readback for seq 11, and
  reproduced answer score bytes exactly at
  `/home/croyse/calyx/data/fsv-issue255-ph36-integration-20260609`.
- **Post-sweep note.** Stage 7 exit #256 rolls PH35-PH36 into one evidence
  bundle. Aiwonder readback at
  `/home/croyse/calyx/data/fsv-issue256-stage7-exit-20260609-nomock` proves 100
  Aster writes with 100 chained ledger rows, failure-atomic group commit,
  all 10 `EntryKind` values, append-only delete/tombstone rejection,
  physical redaction grep clean, three Admin checkpoint rows, seq 11
  quarantine after tamper, reproduce `max_drift=0.0` through real
  `calyx-registry` lenses and `calyx-forge` TurboQuant determinism, and
  complete/partial audit trace behavior. Reference hashes: `stage7-exit.log`
  `3c9b2e9d5ca2c925bca52f6d0d0f3fcf0377900e56728c0c85f3c2e81505ad5e`,
  `ledger-appender-readback.json`
  `f6bc6713f91eb93be892c468c09deaa57678ece07f1ffcea77018758e9b72299`,
  `ph36-fsv-integration-readback.json`
  `c53bda82248727fe8f79334a2cf180890082929153e429bebae2ab1ce779af57`,
  and `audit-query-readback.json`
  `153aab69eabd70801d7d7c7a542dc46178a189c1ec340487a6dd9dfef51a52f2`.
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

**Status:** DONE / FSV-signed-off through #256 plus post-exit PH36 hardening
#349 and #651. Stage 8 Ward is also signed off through #280. #349 proves
filtered audit queries ignore unrelated quarantined rows outside the requested
result set, still fail closed for requested ranges or matching/relevant
quarantined rows, reject physical ledger row-key mismatches, and use typed `cx`
provenance fields instead of arbitrary payload string matching. #651 proves
verify-chain quarantines missing, truncated, and key/encoded-sequence-mismatched
physical Ledger rows. Evidence roots:
`/home/croyse/calyx/data/fsv-issue349-audit-query-hardening-20260609-5697553`
and `/home/croyse/calyx/data/fsv-issue651-verify-chain-physical-20260610`.
