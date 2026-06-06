# Stage 19 — Leapable Vault Swap (PH71)

The shippable customer value: replace the end-user SQLite/`sqlite-vec` Vaults
with embedded Calyx — multi-lens, kernel-grounded, guarded, provenanced — while
Leapable's **PostgreSQL control plane stays untouched**. This is the only
*required* Leapable phase; Discover-Vault hosting (V3) is optional. Scope is
locked and narrow (PRD `15`).

> **Hard boundary (load-bearing):** Calyx replaces only the Vault. It does not
> read, write, replace, shadow, or migrate PostgreSQL. The Vault adapter must
> implement the existing `vault-sqlite.ts` contract so the PG control plane sees
> no behavioral change. On aiwonder, **do not touch** the existing
> leapable/postgres state (`01 §2`).

---

## PH71 — V0 shadow → V1 flip → V2 calyx-only
- **Objective.** Migrate the Vault format with FSV gates at each step, zero
  control-plane involvement.
- **Deps.** PH64 (migration), PH33 (kernel), PH38 (guard), PH63 (MCP).
- **Deliverables.**
  - **V0 shadow:** `libcalyx` embedded as a shadow index beside `sqlite-vec`;
    ingest writes both; Ask reads `sqlite-vec`, compares Calyx. Gate: recall
    parity ≥ baseline on a real Vault, byte-readback.
  - **V1 flip:** Vault reads → Calyx; `sqlite-vec` becomes shadow; multi-lens
    panel + kernel/guard enabled. Gate: A/B recall win, no latency regression;
    `calyx migrate vault` round-trips a real `.db` byte-exact on content.
  - **V2 calyx-only:** remove the `sqlite-vec` shadow; default panels per Vault
    type. Gate: a real production Vault runs Calyx-only with full provenance,
    verified by readback; **control-plane queries/billing/listing for that Vault
    return identical results**.
- **Key tasks.** preserve `database_name`/`chunk_id`/SQL-contract names; embedded
  backend needs no services (CPU SIMD Forge + ONNX lenses); never persist
  candidate text; never mix vectors across models (LensId content-addressing).
- **FSV gate.** the three sub-gates above, each proven by **byte readback** on a
  real Vault on aiwonder; PostgreSQL verified untouched (its responses identical
  before/after).
- **Axioms/PRD.** P11, `15 §1/§4/§5`, A18.

---

## Stage 19 exit
Every Leapable Vault is a multi-lens, kernel-grounded, guarded, provenanced
constellation store (V0→V1→V2), `sqlite-vec` retired, and Leapable's PostgreSQL
control plane is exactly as it was and never touched — PRD `LEAPABLE`. This
alone justifies the project (PRD `19 §2`).
