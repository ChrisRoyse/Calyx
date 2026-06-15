# 30 — Security, Hardening, Privacy & Governance

> **Living-system role:** immune system + integrity — defend the boundary, protect what's inside, honor the rights of whose data it is (A16/A2 — DOCTRINE §0)

Implements new **A33 (security & privacy by construction)**. Ward (`09`) guards *generated vectors* against drift/injection; this doc covers the rest a real database holding everyone's data needs: the STRIDE threat model, the hardening axes, authz/authn, encryption, tenant isolation, supply chain, and — critically — **privacy/right-to-erasure**, which resolves a latent contradiction with A25. Grounded in `AICodingAgentSuperPrompt.md` §14 + Leapable security posture; strictly engineering (DOCTRINE §2).

## 1. Threat model (STRIDE)

| Threat | Surface | Defense |
|---|---|---|
| **Spoofing** | who is calling `calyxd` / which vault | mTLS / Cloudflare Access tokens (`16`); per-vault identity; no anonymous writes (fail-closed, A16) |
| **Tampering** | data at rest, the Ledger, an index | ZFS checksums + scrub (bit rot); Ledger hash-chain + Merkle (`11`); rebuildable indexes; content-addressed lens/codebook/panel (immutable) |
| **Repudiation** | "I didn't do that" | append-only Ledger, actor-stamped, signed Merkle export (`11`) |
| **Information disclosure** | cross-vault leak, secret leak, candidate-text persistence | default-deny tenant isolation (§3); secrets in Infisical, never in repo/issue (`16 §5b`); reranker candidate text request-scoped; redacted-input provenance (hash-only) |
| **Denial of service** | flood ingest/query, OOM, compaction storm | bounded queues + backpressure + admission control (`24 §6`); rate limits; disk-pressure guard; per-tenant quotas |
| **Elevation of privilege** | escape a vault, run code via a lens | least privilege (§2); lenses run sandboxed (`05` runtimes); no `external-cmd` lens without an allowlist; fail-closed on unknown principal |

## 2. Hardening axes (mapped to Calyx, from the 14-axis reference)

| Axis | Calyx stance |
|---|---|
| **AuthN** | server: mTLS / Cloudflare Access service tokens; embedded: in-process (the host app owns identity). No anonymous mutation. |
| **AuthZ** | per-vault capability grants; cross-vault read requires an explicit grant; default deny (A16). Lens-store shared by content-id but **vectors/constellations never cross tenant** (`03 §7`). |
| **Crypto at rest** | ZFS native encryption on `hotpool`/`archive` calyx datasets; **per-vault encryption key** for embedded vaults (the host app's key); codebooks/Ledger encrypted with the vault. |
| **Crypto in transit** | TLS only; loopback + Cloudflare Tunnel for server ingress (`16`); never plaintext over the wire. |
| **Input validation** | validate at boundaries (lens output shape/finite, query shape, record schema); fail-closed structured errors (`18`); never trust an external lens endpoint's bytes blindly (NaN/dim guards, A16). |
| **Secrets** | Infisical only; env-var names in code; pre-commit secret scan (Leapable pattern); never a value in repo/issue/chat (`16 §5b`). |
| **Logging/audit** | the Ledger *is* the audit log (`11`); every mutation/answer/guard/anneal/erasure entry is hash-chained + actor-stamped. |
| **Rate limiting / quotas** | per-tenant ingest/query quotas + backpressure (`24 §6`). |
| **Dependency / supply chain** | pin crate versions + `cargo audit` run locally/on aiwonder; content-addressed lens weights (a swapped model = a new `LensId`, detectable); reproducible builds; SBOM. |
| **Least privilege** | `calyxd` runs as non-root `leapable`, loopback-bound, secrets `0400` (`16`); lenses sandboxed; Anneal capped. |
| **Fail closed** | every unknown/over-budget/corrupt/ungrounded-trusted path errors, never a silent fallback (A16). |
| **Defense in depth** | no single control trusted; e.g. tenant isolation = key separation **and** keyspace separation **and** grant checks. |

## 3. Tenant isolation (multi-tenant `calyxd`)

- **One vault = one tenant boundary.** Per-vault keyspace, per-vault encryption key, per-vault write lock. A query/transaction cannot read another vault's bytes without an explicit, Ledger-logged grant (default deny).
- **Shared lens store, never shared vectors:** many vaults reference the same `LensId` (weights/codebook on disk once), but every constellation/vector is vault-scoped and encrypted per-vault (`03 §7`). Content-id sharing never leaks content.
- **Noisy-neighbor:** per-tenant compute/VRAM/IO quotas; Anneal yields; a heavy tenant cannot starve another (`24`).
- FSV: attempt cross-vault read without a grant → denied + audited; verify another tenant's bytes are unreadable (encrypted) even with raw disk access.

## 4. Privacy, governance & right-to-erasure (resolves the A25 tension)

**The clarification (binding):** A25 ("never delete data, no intelligence lost") forbids **deleting data *to achieve compression*** — you compress the representation, you don't drop data to save space. **A25 does NOT forbid policy- or user-driven deletion.** Erasure, retention, and right-to-be-forgotten are **first-class, explicit operations** and always honored. The two never conflict: compression never deletes; governance deletes on purpose, by request or policy.

| Concern | Mechanism |
|---|---|
| **Right to erasure (GDPR/CCPA)** | `erase(vault | cx | subject)` — a first-class op: removes the constellation(s) + derived (cross-terms, index entries, recurrence occurrences) and **crypto-shreds** the per-vault/per-record key so even cold/backup copies and the append-only Ledger payloads become unrecoverable (the Ledger keeps a *tombstone* entry — "erased at T by actor" — for audit, with no recoverable content). |
| **Append-only Ledger vs erasure** | Ledger stores **hashes/ids, not content** (`11`); erasure crypto-shreds the keyed content and writes an erasure tombstone — provenance of *that an erasure happened* survives; the *content* does not. |
| **Retention / data minimization** | per-collection TTL/rollup (`24`/`25`); recurrence series downsample old occurrences; cold-tier then purge on policy. |
| **Consent & purpose** | anchors/inputs carry a purpose/consent tag; a vault declares its lawful basis; processing that exceeds consent fails closed. |
| **Data residency** | a vault pins its storage location (aiwonder ZFS dataset); no off-box copy without policy (single-host posture, `16`). |
| **PII redaction** | raw input may be hash-only / redacted while vectors persist (`03`/`11`); reranker candidate text never persisted. |
| **De-identification** | constellations can be stored without raw input (hash-only), so search/intelligence works on vectors with the PII source removed. |

So: **Calyx never deletes to compress (A25), and always deletes when law or the user requires it (A33)** — via crypto-shredding that satisfies erasure even against an append-only log and backups. This must be stated in code reviews so no agent ever refuses a lawful delete citing A25.

## 5. Cold-start / bootstrap (a vault before it has anchors)

A new vault has no anchors → bits/kernel/guard are `provisional` (A2). The bootstrap path:
1. **Day 0 — search works immediately.** Default panel + multi-lens RRF retrieval needs no anchors; the vault is useful for search/navigation from the first ingest.
2. **Provisional everything else.** Assay/Lodestar/Ward run but tag outputs `provisional`; high-stakes paths refuse provisional (A2). `grounding_gaps` lists the cheapest anchors to label.
3. **First anchors → trust turns on.** As real outcomes arrive (test pass, thumbs, label), bits/kernel/τ calibrate; `J` starts to climb (`27`). The intelligence-gradient (`27 §3`) front-loads the highest-info anchors to ground fastest.
4. **Never fake grounding.** A vault that never gets anchored stays in provisional-search mode honestly — Calyx says so (it does not invent confidence). This is the honest cold-start.

## 6. The axiom & build gate

- **A33 — Security & privacy by construction.** Least privilege, default-deny tenant isolation, encryption at rest + in transit, input validation, supply-chain integrity, the Ledger as audit; **right-to-erasure is first-class and always honored via crypto-shredding** (A25 forbids deleting-to-compress, never lawful/user deletion); fail closed on every security/privacy uncertainty (A16).

```
SECURITY := STRIDE defenses FSV-proven ∧ cross-vault read denied+audited ∧ at-rest+in-transit encryption verified
          ∧ erase() crypto-shreds (content unrecoverable incl. backups/Ledger payload; tombstone remains) ∧ secret-scan clean
```
Added to `BUILD_DONE` (`19`). FSV each: e.g. erasure → after `erase`, read raw disk + backup + Ledger and prove no recoverable content, tombstone present.

**One sentence:** Calyx is secured by construction — STRIDE-modeled, least-privilege, encrypted at rest and in transit, default-deny tenant isolation with per-vault keys, supply-chain-pinned, the Ledger as a tamper-evident audit log — and governed by construction: right-to-erasure is a first-class crypto-shredding operation that A25 explicitly permits (A25 only forbids deleting *to compress*), so Calyx never loses intelligence to save space yet always honors a lawful or user-requested deletion, and a fresh vault bootstraps honestly (search from day 0, trust as anchors arrive, never faked).
