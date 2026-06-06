# Stage 12 — Universal Data Layer (PH53–PH55)

Make Calyx a real general-purpose database: serve the root purpose of every
paradigm on the Aster core, with intelligence opt-in per collection (progressive
enhancement). Storage-shaped paradigms are the general data layer; search-shaped
ones are already the Association Engine. Lands in `calyx-aster` (layers) +
`calyx-sextant` (query). *Can proceed in parallel with S5–S8 once Aster (S1)
exists.*

---

## PH53 — Collections-as-any-model (relational/doc/KV/TS/blob)
- **Objective.** A `Collection { mode, schema?, panel?, dedup, temporal, … }`
  that behaves as any data model; 0 lenses = plain store, ≥1 lens = constellations.
- **Deps.** PH09.
- **Deliverables.** key-encoding layers over Aster: relational `(table,pk)→row`
  + `(idx,val,pk)→∅`; document tuple-path keys + subtree prefix-read; KV
  `(ns,key)→val`+TTL; time-series `(series,ts)→point`+rollups; blob chunked
  payload+manifest. `create_collection`, `put_record`/`get_record`/`range`.
- **Key tasks.** each paradigm's root op (`20 §2`); schema (SchemaFull|
  SchemaLess); dedup/temporal policy at creation (`25`).
- **FSV gate.** each paradigm's root op (point/range/join-by-ref/aggregate/
  traverse/rollup) round-trips by readback on aiwonder; a 0-lens collection is a
  plain fast store; `add_lens` upgrades it to constellations.
- **Axioms/PRD.** A19, `20 §2/§3`, `04 §2`.

## PH54 — Secondary indexes (btree/inverted)
- **Objective.** Indexing = write the data key **and** the index key in one
  transaction (FoundationDB pattern).
- **Deps.** PH53.
- **Deliverables.** `index/btree` (scalars/range), `index/inverted` (reuse PH25),
  ANN + kernel indexes already exist; secondary-index maintenance in the write
  txn.
- **Key tasks.** atomic data+index write; range/point queries; index rebuild
  (self-heal).
- **FSV gate.** an index key is written in the **same txn** as its data key
  (read both at one seq); range + point queries correct; a crash leaves no
  half-indexed row.
- **Axioms/PRD.** `20 §1/§2`, A15.

## PH55 — Cross-model transactions + universal query surface
- **Objective.** One statement, one transaction, across modes — what used to need
  five systems.
- **Deps.** PH54, PH26.
- **Deliverables.** the Sextant universal surface (`10 §0`): typed predicates +
  graph traversal + multi-lens vector/FTS fusion + OLAP aggregate + TS range +
  `ASK`; single-writer-per-vault serialization; declared isolation; planner cost
  caps.
- **Key tasks.** cross-collection atomic write; one query spans modes
  (relational filter → graph hop → vector similarity → aggregate) in one pass;
  `ASK` = multi-lens + kernel_answer + Oracle, grounded + provenanced.
- **FSV gate.** one txn touching relational + constellation + graph collections
  → **no partial read, no deadlock** (consistent seq, read it); a cross-model
  query returns one provenanced result set in one pass; an unbounded plan is
  rejected.
- **Axioms/PRD.** A19, `20 §4/§8`, `17 §7.3`.

---

## Stage 12 exit
Calyx is one engine that is every database a project needs — relational/doc/KV/
columnar/graph/TS/FTS/vector/blob on one ordered transactional core, with the
Association Engine subsuming the search-shaped ones and adding DDA/kernel/`Gτ`/
Oracle — PRD `UNIVERSAL`. A greenfield project can put everything in Calyx.
