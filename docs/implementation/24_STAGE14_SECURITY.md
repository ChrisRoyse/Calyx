# Stage 14 — Security & Privacy by Construction (PH60–PH61)

STRIDE-modeled, least-privilege, encrypted at rest + in transit, default-deny
tenant isolation, supply-chain-pinned, Ledger-as-audit — and right-to-erasure as
a first-class crypto-shredding operation that resolves the A25 tension.
Cross-cutting — apply continuously; finalized here. **Living-system role:**
immune system + integrity. Strictly engineering (DOCTRINE §2).

---

## PH60 — Encryption at rest/in transit + tenant isolation
- **Objective.** Per-vault keys + keyspace + grant checks; ZFS-native encryption;
  TLS only; one vault = one tenant boundary, default deny.
- **Deps.** PH09.
- **Deliverables.** ZFS native encryption on the calyx datasets (operator/sudo
  to enable); per-vault encryption key (embedded = host app key); TLS/mTLS or
  Cloudflare Access for `calyxd`; per-vault keyspace + write lock; grant model
  (cross-vault read requires explicit, Ledger-logged grant); shared lens store
  by content-id but vectors never cross tenant.
- **Key tasks.** `CALYX_VAULT_ACCESS_DENIED` on ungranted cross-vault read;
  defense in depth (key + keyspace + grant); per-tenant quotas (noisy neighbor).
- **FSV gate.** cross-vault read without a grant → **denied + audited** (read the
  Ledger); another tenant's bytes are **unreadable even with raw disk access**
  (encrypted — verify on aiwonder).
- **Axioms/PRD.** A33, A16, `30 §1/§2/§3`, `03 §7`.

## PH61 — Crypto-shred erasure + STRIDE FSV + secret-scan
- **Objective.** Right-to-erasure as a first-class op (A25 forbids deleting-*to-
  compress*, never lawful/user deletion); the full STRIDE defense set FSV-proven.
- **Deps.** PH60, PH36 (Ledger).
- **Deliverables.** `erase(vault|cx|subject)` — remove constellation(s) + derived
  + recurrence occurrences and **crypto-shred** the per-vault/per-record key so
  cold/backup/Ledger-payload copies become unrecoverable; the Ledger keeps an
  **erasure tombstone** (no recoverable content); retention/TTL; consent/purpose
  tags; PII redaction (hash-only input). Supply chain: pinned crate versions +
  `cargo audit`, content-addressed lens weights, SBOM. Secret-scan (pre-commit).
- **Key tasks.** erasure satisfies GDPR/CCPA even against the append-only log +
  backups; **no agent may refuse a lawful delete citing A25**; cold-start
  honesty (search day 0, trust as anchors arrive, never faked).
- **FSV gate.** after `erase`: read raw disk + backup + Ledger payload → **no
  recoverable content, tombstone present** (verify on aiwonder); cross-vault
  read denied+audited; at-rest+in-transit encryption verified; secret-scan clean.
- **Axioms/PRD.** A33, A25, A2, `30 §4/§5/§6`.

---

## Stage 14 exit
Calyx is secured + governed by construction — STRIDE-modeled, least-privilege,
encrypted, default-deny isolation, supply-chain-pinned, tamper-evident audit —
and never loses intelligence to save space yet always honors a lawful/user-
requested deletion via crypto-shredding, bootstrapping honestly — PRD `SECURITY`.
