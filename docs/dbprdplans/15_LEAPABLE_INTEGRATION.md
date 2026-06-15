# 15 — Project Profile: Leapable (Vault Replacement Only)

Implements A18. **Leapable is one project using the universal Calyx engine (`20`); this doc is its deployment profile.** Scope is locked and narrow: Calyx replaces the end-user SQLite/`sqlite-vec` Vaults; the PostgreSQL control plane is OUT OF SCOPE and untouched. (A greenfield project could use Calyx as its sole database — general data layer + Association Engine — per `20 §6`. "Keep PostgreSQL" is specific to Leapable's mature production system, not a Calyx limitation.)

## 1. The boundary (read this first)

| Layer | Today | After Calyx |
|---|---|---|
| **End-user Vault** (`vault-sqlite.ts` + `sqlite-vec`, bundled in the Tauri sidecar) | SQLite, single GTE 768-d lens | **embedded Calyx** (`libcalyx`) — multi-lens constellation store. **This is the only thing that changes.** |
| **Central control plane** (PostgreSQL 18 + PgBouncer on aiwonder: `central-postgres.ts`, `creator_databases`, `queries`, billing, outbox, marketplace, customer info) | PostgreSQL | **UNCHANGED.** Calyx does not read, write, replace, shadow, or migrate PostgreSQL. It stays the source of truth for all control-plane state, accessed exactly as today. |
| Published/Discover Vault hosting on aiwonder | served from the box | optionally a `calyxd` **Vault host** (still just Vaults; still does not touch PostgreSQL) |

Leapable's invariant *"Vault = SQLite, aiwonder central backend = PostgreSQL"* becomes *"**Vault = Calyx**, aiwonder central backend = PostgreSQL (unchanged)."* Only the left half moves.

This scope deliberately **removes the single highest-risk item** from the original plan (replacing a mature multi-writer RDBMS control plane). What remains is a contained, high-value storage-format swap.

## 2. Why a Vault is the right thing to replace

A Vault holds the calculus-of-association value: the user's Sources/chunks and their vectors. Today that is one embedding (GTE 768-d) in `sqlite-vec`. Calyx turns every Vault into a multi-lens constellation store — **without involving the control plane at all**, since a Vault is self-contained local state.

| Vault concern today | Calyx object |
|---|---|
| `chunks` + `sqlite-vec` 768-d vector | Constellation (1-slot → N-slot panel) |
| Vault-local provenance/citations/audit | Ledger (`11`) |
| Vault-local knowledge-graph edges | cross-term agreement/delta graph + entity lens (`06`) |
| reranker confidence | scalar + `Gτ` reading (`09`) |
| `chunk_id`, `database_name` identifiers | preserved verbatim (code-contract names unchanged) |

## 3. End-user capabilities unlocked (the product win)

Through the local MCP sidecar, every Vault gains what used to take a research team:

| Capability | User-facing |
|---|---|
| Multi-lens Ask | answers fuse semantic + keyword + code + entity + causal lenses, not one embedding |
| Add a lens | "view my Vault through a legal / code / commissioned lens" — one action, no re-embed |
| The core (kernel) | "show me the 1% of my Vault that explains the rest" |
| Bits | "which lenses actually help on my data" |
| The boundary (`Gτ`) | "keep my AI's answers inside what my Sources support" — injection/hallucination guard |
| Provenance | every answer cites its grounded Sources, replayable (`reproduce()`) |
| Self-tuning | the Vault gets faster/sharper the more it's used |

Marketplace, billing, Discover listing, and creator metadata keep flowing through PostgreSQL exactly as before — Calyx just makes each Vault's *contents* smarter.

## 4. Invariants to preserve (Leapable rules Calyx must honor)

| Leapable invariant | Calyx stance |
|---|---|
| End users run the signed Tauri sidecar, no Docker/Node/Python | `libcalyx` is a static Rust lib in the sidecar; embedded backend needs no services (CPU SIMD Forge; ONNX lenses) |
| Embedding centralized on aiwonder (resident GTE TEI) for cloud paths | server/Discover Vaults use `tei-http` lenses to `:8088/:8090/:8089`; **never mix vectors across models** → enforced by `LensId` content-addressing (`03`) |
| Backend storage = POSIX-on-ZFS, no S3/Tigris/B2 | Aster is POSIX files (`04`); no object store |
| Secrets in Infisical; never persist candidate text | Ledger stores hashes not secrets; redacted-input provenance (`11`); reranker candidate text stays request-scoped |
| Vault = the local source of truth | embedded Calyx Vault is the SoT; control plane unchanged |
| `database_name`, `/api/databases/*`, `leapable_db_*`, `chunk_id`, SQL table names in code contracts | unchanged — Calyx sits behind the same Vault interface the control plane already calls |

**Key point:** the control plane talks to a Vault through a storage interface (`vault-sqlite.ts`'s contract); Calyx implements that same contract. The PostgreSQL side never knows the Vault's bytes changed from SQLite to Aster — it still gets the same `database_name`/`chunk_id`/query responses.

## 5. Migration plan (Vault-only, FSV-gated)

| Phase | Scope | Gate |
|---|---|---|
| **V0** | Ship `libcalyx` embedded as a **shadow** index alongside `sqlite-vec`; ingest writes both; Ask reads `sqlite-vec`, compares Calyx | recall parity ≥ baseline on a real Vault, byte-readback |
| **V1** | Flip Vault reads to Calyx; `sqlite-vec` becomes shadow; enable the multi-lens panel + kernel/guard for users | A/B recall win, no latency regression; `calyx migrate vault <sqlite> <vault.calyx>` round-trips a real `.db` byte-exact on content |
| **V2** | Remove the `sqlite-vec` shadow; Calyx is the sole Vault engine; default panels per Vault type | a real production Vault runs Calyx-only with full provenance, verified by readback |
| **V3** (optional) | `calyxd` hosts published/Discover Vaults on aiwonder (still no PostgreSQL involvement) | Discover serves a Calyx-backed Vault; control-plane listing/billing unchanged and verified |

There is **no control-plane phase.** PostgreSQL is never dual-written, shadowed, or flipped. The original plan's L2–L4 (control-plane replacement) are **deleted from scope.**

Migration tooling: `calyx migrate vault <sqlite> <vault.calyx>` (chunks → 1-slot constellations, then lazy panel backfill). It verifies by **direct row/byte readback** of constellations vs source SQLite rows, not a harness (Leapable doctrine). The `vault-sqlite.ts` allowed-direct-import becomes the Calyx Vault adapter; its tests are preserved/ported.

## 6. Deployment shape

- **Embedded (primary):** `libcalyx` linked into the Tauri sidecar; one `vault.calyx` dir per user DB; MCP over stdio; CPU SIMD Forge + optional ONNX GPU. Replaces the `sqlite-vec` Vault path; no new services on the user machine.
- **Server (optional, `calyxd`):** only if Discover/published Vaults are hosted on aiwonder — a loopback systemd Vault host (`16`) using resident TEI lenses and Aster on ZFS. **Serves Vaults; not a control-plane database; does not connect to PostgreSQL.**

## 7. Risk posture

With control-plane replacement removed, residual risks are contained and listed in `17_JOHARI_BLINDSPOTS.md`: Vault migration byte-fidelity, "never mix vectors across models," embedded GPU-less performance, the universal grounding caveat. The Vault swap (V0–V2) is low-risk and is the recommended first and only Leapable milestone needed to ship real value.

**One sentence:** Calyx becomes the engine **inside each Leapable Vault** — replacing SQLite/`sqlite-vec` with a multi-lens, kernel-grounded, guarded, provenanced constellation store — while Leapable's PostgreSQL control plane (customers, billing, creators, queries) stays exactly as it is and is never touched.
