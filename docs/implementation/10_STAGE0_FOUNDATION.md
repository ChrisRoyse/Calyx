# Stage 0 — Foundation & Environment (PH00–PH04)

> **STATUS: ✅ DONE** (2026-06-07, commit `8dcddaa`). The self-contained Calyx
> environment is up on aiwonder, the workspace compiles, the five pinned
> `type:context` issues are live (#22–#26), and `calyx-core` ships every ID,
> enum, `CALYX_*` error code, model struct, trait, and the injected `Clock`.
> This was the first stage built; it has no per-phase card subdir (cards cover
> PH05→PH72, see `PHASE_TASKS_README.md`).

Stood up the self-contained Calyx environment on aiwonder, the Rust workspace,
the dev-state surface, and the dependency-free `calyx-core` types every other
crate builds on.

Exit of Stage 0 (achieved): an agent on aiwonder can `cargo test` a green
workspace whose `calyx-core` defines every ID, enum, error code, core struct,
and trait the PRD names — with the ≤500-line gate passing and the context issues
live.

---

## PH00 — aiwonder bootstrap & self-contained Calyx home

**Objective.** Create `CALYX_HOME=/home/croyse/calyx` and prove the box can
build Calyx, touching nothing else.

**Deps.** none (VPN up; `.env` filled).

**Deliverables.**
- `CALYX_HOME` tree (`repo/ target/ data/ datasets/ .hf-cache/ logs/ tmp/ bin/`).
- `repo/env.sh` — sources `~/.cargo/env`, exports `CALYX_*`,
  `CALYX_CARGO_TARGET_DIR`, `HF_HOME`, CUDA paths, the aiwonder-built ONNX
  Runtime CUDA 13 dynamic library path, prepends `bin/` + `~/.local/bin` to
  PATH, and clears inherited `CARGO_TARGET_DIR` values that point outside
  `CALYX_HOME`.
- A recorded **system baseline** (FSV readback file) in the repo planning notes.
- Userspace `cmake` + `protoc` in `CALYX_HOME/bin` (no sudo).

**Key tasks.** (see task cards T-001…T-005)
- Verify SSH + record GPU/CPU/RAM/CUDA/ZFS readback.
- Create the home tree; write `env.sh`; reuse rustup, isolate target dir.
- Smoke-test: `rustc --version`, `nvcc --version`, a 1-line cudarc/candle GPU
  probe compiles for sm_120.
- (Operator/sudo, non-blocking) create `hotpool/calyx` + `archive/calyx`
  datasets; else run from home and relocate later.

**FSV gate.** `ls`/`zfs list` show the home (and datasets if created) exist;
`cargo`/`nvcc`/`nvidia-smi` readback printed and attached; a hello-world crate
builds **and runs on aiwonder** with output in `CALYX_HOME/target`; no file
created outside `CALYX_HOME`.

**Axioms/PRD.** §8c (everything on aiwonder), `16`, `28 §5`. **Risks.**
password-backed sudo is gated and must never print the secret value; ZFS/systemd
are deferred until their owning phases. `cmake` and `protoc` are now installed
under `CALYX_HOME/bin` and verified in `01 §2/§6`.

---

## PH01 — Rust workspace + crate skeletons + line-count gate

**Objective.** A cargo workspace with every Calyx crate as a compiling skeleton,
plus the ≤500-line gate wired as a pre-merge script.

**Deps.** PH00.

**Deliverables.**
- `repo/Cargo.toml` (workspace), `rust-toolchain.toml` (pin the channel),
  `.cargo/config.toml` (sm_120 / target tuning if needed).
- Crate skeletons: `calyx-core calyx-aster calyx-forge calyx-registry
  calyx-loom calyx-assay calyx-lodestar calyx-mincut calyx-paths calyx-ward
  calyx-sextant calyx-ledger calyx-anneal calyx-oracle calyx-mcp calyx-cli
  calyxd` — each `lib.rs`/`main.rs` with a doc header + a trivial test.
- `repo/scripts/linecount.sh` (the gate), `repo/scripts/check.sh`
  (check+clippy+test+gate wrapper).

**Key tasks.** workspace members; shared deps via `[workspace.dependencies]`;
pin toolchain; wire the gate; one passing test per crate.

**FSV gate.** `cargo check --workspace` + `cargo clippy -D warnings` +
`cargo test --workspace` green **on aiwonder**; `linecount.sh` prints ✅.

**Axioms/PRD.** §8 (≤500 lines), `18 §1` (crate layout), A34 (free OSS).

---

## PH02 — GitHub repo + pinned context issues + workflow

**Objective.** The `chrisroyse/calyx` repo + the five pinned `type:context`
issues every agent reads each turn.

**Deps.** PH00 (uses `gh` on aiwonder, already authed; else auth).

**Deliverables.**
- Repo created/pushed; `.gitignore` (already present), `README`, the
  `docs/` planning tree committed.
- Labels: `type:context|task|decision|discovery|blocker`,
  `status:in-progress|blocked`, `area:*` per engine, `p0`–`p3`.
- Five pinned `type:context` issues: Mission & invariants · You-are-here ·
  Environment & ops · Landmines · Datasets (bodies = pointers to docs, short,
  last-verified stamp — PRD `29 §2`).

**Key tasks.** create repo; push; create labels; open + pin the five issues
with the read-state protocol noted; record the landmines (sudo is
password-backed and secret-safe, not passwordless; rust-is-installed
correction; ≤500-line; FSV reads bytes; never secret values
in issues; dedup never merges conflicting anchors).

**FSV gate.** `gh issue list --label type:context` returns exactly the five,
pinned; bodies are pointers not copies; repo clone on aiwonder matches.

**Axioms/PRD.** §8d, `29`.

---

## PH03 — calyx-core: IDs, enums, error catalog

**Objective.** The dependency-free identity + error vocabulary.

**Deps.** PH01.

**Deliverables (in `calyx-core`, each module ≤500 lines).**
- `ids.rs` — `VaultId(Ulid)`, `LensId([u8;16])`, `CxId([u8;16])`,
  `SlotId(u16)`; content-addressing helpers (`blake3` of canonical inputs);
  stable serde + `Display`/`FromStr`.
- `enums.rs` — `Modality`, `SlotShape`, `Asymmetry`, `QuantPolicy`,
  `AnchorKind` (incl. `SpeakerMatch`/`StyleHold`/`Recurrence`), `SlotState`,
  `AbsentReason`.
- `error.rs` — `CalyxError { code, message, remediation }` with **every**
  `CALYX_*` code from `dbprdplans/18 §6` as variants; `Result<T> = …`.

**Key tasks.** implement; proptest round-trips (`decode(encode(x))==x`) for IDs;
enumerate error codes as a closed set; no I/O, no deps beyond `blake3`/`ulid`/
`serde`.

**FSV gate.** unit+proptest green; an enumerated list of `CALYX_*` codes
printed and matched against `18 §6`; ID content-addressing is deterministic
(same input → same 16 bytes, verified by readback).

**Axioms/PRD.** A1, A16, `03 §2`, `18 §2/§6`.

---

## PH04 — calyx-core: core structs + traits

**Objective.** The constellation data model + the engine trait boundaries.

**Deps.** PH03.

**Deliverables (in `calyx-core`).**
- `model.rs` — `Constellation`, `Slot`, `Panel`, `Anchor`, `SlotVector`
  (`Dense|Sparse|Multi|Absent`), `Signal`, `CxFlags`, `InputRef` (split into
  submodules to stay ≤500 lines).
- `traits.rs` — `Lens`, `Index`, `VaultStore`, `Estimator` (signatures from
  `18 §3`), all returning `Result<_, CalyxError>`.
- `time.rs` — a `Clock` trait (injected; never `SystemTime::now()` in logic) +
  monotonic server-stamp type.

**Key tasks.** implement; serde round-trip byte-exact (proptest); `SlotVector`
`Absent` is explicit (no zero-fill, A16); `Clock` trait enables deterministic
tests.

**FSV gate.** serde round-trip of a `Constellation` is byte-exact; an `Absent`
slot never materializes a zero vector; traits compile and are object-safe where
needed; the whole `calyx-core` test suite green on aiwonder.

**Axioms/PRD.** A1, A3, A4, A16, `03 §3/§4`, `18 §2/§3`.

---

## Stage 0 exit checklist — ✅ all met
- [x] `CALYX_HOME` self-contained; nothing written outside it (PH00).
- [x] Workspace compiles + lints + tests + ≤500-line gate green (PH01).
- [x] Five pinned context issues live; read-state query returns them (PH02, #22–#26).
- [x] `calyx-core` IDs/enums/errors/structs/traits done, proptest round-trips
      byte-exact, error codes match `18 §6` (PH03–PH04).
- [x] Stage-0 sign-off recorded with FSV evidence (readback output).
