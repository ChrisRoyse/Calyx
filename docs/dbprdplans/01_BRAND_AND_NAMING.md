# 01 — Brand & Naming

## 1. The name: **Calyx**

> A *calyx* is the whorl of sepals that encloses a flower in bud and holds the bloom together at its base — the grounded structure from which the constellation of petals opens.

**Calyx** holds the constellation. Every record is a Teleological Constellation (a bloom of vectors); the calyx is the grounded base — kernel, provenance, structure — that holds it and lets it open into search, naming, answer.

### Why this name (5 load-bearing reasons)

| # | Reason |
|---|---|
| 1 | **Phonetic anchor to the thesis.** *Calculus* of association → **Calyx**. The name *is* the theory's first word, compressed. |
| 2 | **Exact metaphor.** Calyx = the grounded holder of a constellation of petals. The DB = the grounded holder (kernel + provenance) of a constellation of slot-vectors. The metaphor is structural, not decorative. |
| 3 | **Organic / self-growing.** The paper grounds intelligence in the mental lexicon, which *"grows by differentiation"* (Steyvers & Tenenbaum). Calyx self-heals, self-prunes lenses, grows new constellations. A botanical mark fits a living system better than a mechanical one. |
| 4 | **Ownable & clean.** Two syllables, unambiguous spelling, no negative connotations (cf. "Prism"/PRISM surveillance), uncrowded in the database namespace, Rust-crate friendly (`calyx-*`), CLI friendly (`calyx`). |
| 5 | **Visual leverage.** A calyx is a radial 5-point form that reads simultaneously as a sepal-whorl, a **star/asterism**, and an **asterisk `*`** (the glob/everything operator). One glyph carries flower + constellation + "all data". |

### Pronunciation & forms
- Say: **KAY-liks** (`/ˈkeɪlɪks/`).
- Product: **Calyx**. Engine/binary/crate prefix: `calyx`. Plural of the record: *constellations* (never "calyxes").
- Possessive voice: "in Calyx", "a Calyx vault", "the Calyx panel".

---

## 2. Subsystem naming — the celestial-navigation system

Calyx's engines share one metaphor: **instruments for navigating by stars.** A frozen lens is a sighting instrument; the kernel is the guiding star; search is navigation. Memorable and self-documenting.

| Subsystem | Codename | Meaning of the name | Crate |
|---|---|---|---|
| Embedder registry | **Registry** | plain, intentional | `calyx-registry` |
| On-disk format | **Aster** | Greek *astēr*, "star" — the file holds constellations | `calyx-aster` |
| DDA cross-term engine | **Loom** | weaves associations-between-associations | `calyx-loom` |
| Signal/bits engine | **Assay** | to measure the quality/content of a sample | `calyx-assay` |
| Kernel finder | **Lodestar** | the guiding star you orient everything by = the ≈1% kernel | `calyx-lodestar` |
| `Gτ` guard | **Ward** | a guard / protective boundary | `calyx-ward` |
| Search & navigation | **Sextant** | the instrument for fixing position by the stars | `calyx-sextant` |
| Provenance/witness | **Ledger** | append-only tamper-evident record | `calyx-ledger` |
| Self-optimization | **Anneal** | heat-and-settle into a lower-energy (faster, truer) state | `calyx-anneal` |
| Math/GPU runtime | **Forge** | where the raw metal (matmul, kernels) is worked | `calyx-forge` |
| Core types/traits | — | — | `calyx-core` |
| MCP / agent surface | — | — | `calyx-mcp` |
| CLI | — | binary `calyx` | `calyx-cli` |

**Naming rule (controlled vocabulary).** One word per concept, used everywhere: a frozen embedder is always a **lens** (never "model"/"encoder"/"embedder" in prose), a record is always a **constellation** (never "row"/"doc"/"point"), the bits-about-outcome is always **signal** (never "score"/"weight").

---

## 3. Visual identity

### 3.1 The glyph
A single radial mark with **5 outer points** (sepals/star) converging on **1 filled center point** (the grounding kernel). Reads at 3 scales:
- **16 px favicon:** a 5-point asterisk-star with a bold center dot.
- **App/crate icon:** a calyx — five sepal-leaves opening from a base, negative space forming a star between them.
- **Wordmark lockup:** glyph + `calyx` in lowercase.

The center dot is **always present and emphasized** — the brand's one non-negotiable: *everything is defined relative to the grounded center (the kernel).*

### 3.2 Color
| Token | Hex | Use |
|---|---|---|
| `calyx.ground` | `#0B1020` | near-black blue-ink background (the night sky / the substrate) |
| `calyx.bloom` | `#7C9CFF` | primary — periwinkle/star-blue (the constellation) |
| `calyx.kernel` | `#FFC247` | accent — warm gold (the lodestar/kernel center dot; the one warm point) |
| `calyx.sepal` | `#2BD4A8` | secondary — living green (growth, self-healing) |
| `calyx.signal` | `#FF6B9A` | alert/diagnostic — magenta (low-signal/pruned lens warnings) |
| `calyx.mist` | `#9AA7C7` | muted text on dark |

Palette logic: a dark "sky" ground, cool blue constellations, **one warm gold point** (the kernel) so the eye always finds the center, green for the living/self-growing layer, magenta reserved for diagnostics.

### 3.3 Typography
- **Wordmark / display:** a geometric grotesque with a true single-story `a` and circular `o` (e.g. *Space Grotesk* / *Geist*). Lowercase `calyx`.
- **UI / docs:** *Inter* / system sans.
- **Code / data:** *Geist Mono* / *JetBrains Mono*.

### 3.4 Motion (for site/console)
One signature motion: an input **blooms** — a single dot fans out into a constellation of N points, faint lines draw the cross-terms, dim lines fade, kernel points stay lit gold. Visualizes DDA + kernel in ~1.5 s; the hero animation.

---

## 4. Voice & messaging

**Voice.** Precise, grounded, a little astronomical. Confident about the mechanism, exact about the bounds (the paper states its one caveat plainly; the brand does too). Never mystical in product copy — metaphysics lives in the founder's essays, not the docs.

### Taglines (pick per surface)
- **Primary:** *"Store meaning, not tokens."*
- Technical: *"Every datum, a constellation."*
- Thesis: *"Intelligence is the calculus of association. Calyx is its engine."*
- Developer: *"Multi-embedder search, with the plumbing already built."*
- Founder/vision: *"A grounded substrate for general intelligence."*

### Boilerplate (one paragraph)
> Calyx is a database whose native record is the association-constellation: one input measured through many frozen embedders, fused, differentiated by the bits each adds, and anchored to real outcomes. It finds the small grounding kernel that explains a whole dataset, guards generation against drift, and gets faster the more you use it. Built in Rust with GPU linear algebra baked in. Calyx replaces the SQL stack underneath Leapable.ai and gives every user the multi-lens machinery that used to take a team to build.

### Naming of user-facing concepts (Leapable surface)
- Internal `constellation` → user-facing **"facet bundle"** or kept as **"constellation"** (recommended: keep "constellation"; it's evocative and teachable).
- `lens` stays **lens** (users already grasp "viewing your data through a lens").
- `grounding kernel` → user-facing **"the core"** (the ~1% that explains your data).
- `Gτ guard` → user-facing **"the boundary"** (what your AI is and isn't allowed to say).

---

## 5. Product & artifact names

| Artifact | Name |
|---|---|
| The database | **Calyx** |
| The server daemon | `calyxd` |
| The CLI | `calyx` |
| The embedded library | `libcalyx` / `calyx` crate |
| On-disk file | `*.aster` (a constellation shard), `vault.calyx` (a vault directory marker) |
| The MCP server | **Calyx MCP** (`calyx-mcp`) |
| A user's database | a **Calyx vault** |
| Served published/Discover Vaults on aiwonder | **Calyx Core** (an optional `calyxd` Vault host — does **not** touch the PostgreSQL control plane) |
| Marketing site section | "Calyx — the association engine inside Leapable" |

---

## 6. Alternates (if Calyx is unavailable / disliked)

Ranked, with the same metaphor system available for each:

| Rank | Name | Pron. | Why | Risk |
|---|---|---|---|---|
| A | **Calyx** ✅ | KAY-liks | calculus-link + grounded-holder-of-constellation | mild botanical-product overlap |
| B | **Asterism** | AS-ter-iz-m | literally "a pattern of stars" = a constellation; printing mark ⁂ | 3 syllables, slightly obscure |
| C | **Lodestar** | LODE-star | the guiding star = the kernel; navigation theme | common English word, less ownable |
| D | **Astrolabe** | AS-tro-layb | an instrument that measures position by the stars = the paper's "measurement device" | long; historical-but-known |
| E | **Sidereal** | sy-DEER-ee-al | "of the stars" — measured against the fixed stars | hard to spell/say |

If the user picks B–E, the subsystem codenames (Loom/Assay/Lodestar/Ward/Sextant/Ledger/Anneal/Forge) carry over unchanged — except promote the chosen alternate out of the subsystem list (e.g. if **Lodestar** becomes the product, rename the kernel engine to **Polaris**).

**Recommendation: ship as Calyx.** The only candidate encoding both the *theory* (calculus) and the *architecture* (the grounded calyx holding the constellation) in one ownable, sayable word.
