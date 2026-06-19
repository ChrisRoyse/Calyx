# 01 — aiwonder Environment & the Self-Contained Calyx Layout

Everything Calyx is built/stored/run/tested here. This doc is the **live,
verified** picture of the box (readback 2026-06-18) and the binding rule that
**Calyx is self-contained under one root and touches nothing else.**

---

## 1. Reaching the box (the connect procedure)

1. **VPN up first.** Cisco AnyConnect → `vpn.pcrecruiter.net`, user `sabbey`
   (creds in `../../.env`). Confirmed: AnyConnect adapter is **Up**; the agent
   does not start/stop the VPN — the operator keeps it connected.
2. **SSH.** `ssh croyse@aiwonder.mst.com` (resolves to `68.171.3.249` over the
   tunnel; TCP 22 open; password auth in `.env`).
3. **Non-interactive from Windows** (OpenSSH can't pipe a password) — use the
   `SSH_ASKPASS` mechanism documented at the bottom of `../../.env`. For
   multi-line remote work, base64-encode the script and `| base64 -d | bash`.
4. **Rust/CUDA on PATH:** non-login shells don't have the full Calyx toolchain
   environment. Every remote build/test command should
   `source /home/croyse/calyx/repo/env.sh` first. That entrypoint sources
   Rust, exposes `CALYX_CARGO_TARGET_DIR`, clears any inherited
   `CARGO_TARGET_DIR` outside `CALYX_HOME`, exports the aiwonder-built ONNX
   Runtime CUDA 13 dynamic library path, exports the cuVS/RAFT/RMM/CUDA
   runtime library path, bakes those paths into verified Linux release builds
   as ELF `RPATH`, and pins CUDA discovery to
   `/usr/local/cuda/bin/nvcc` so Rust CUDA build scripts derive
   `/usr/local/cuda/include`, not `/usr/local/include`. Verified builds should
   use `scripts/build-verified-calyx.sh`, which sets and readbacks
   `CARGO_TARGET_DIR` for that command and rejects unresolved ELF dynamic
   dependencies without relying on `LD_LIBRARY_PATH`.

## 2. Verified hardware & OS (live readback 2026-06-18)

| Resource | Value (confirmed) |
|---|---|
| Host / user | `aiwonder` / `croyse` |
| OS | Ubuntu **26.04 LTS**, kernel **7.0.0-15-generic**, systemd, UTC |
| CPU | 32 threads (Ryzen 9-class, 16c/32t) |
| RAM | **121 GiB** total, ~90 GiB available steady-state, **0 swap** |
| GPU | **RTX 5090**, Blackwell, **sm_120 (compute_cap 12.0)**, **32607 MiB**, driver **610.43.02** |
| CUDA toolkit | **13.3** installed (`/usr/local/cuda` → `/usr/local/cuda-13.3`; also 13.2, 13.1, 13.0). `nvcc` V13.3.33 works; `nvidia-smi` reports CUDA UMD 13.3 |
| Root FS | `/dev/nvme0n1p2` ext4, 1.8 TB, **~880 GB free** (holds `/home/croyse`) |
| ZFS hot | `hotpool` 1.81 TB, **~1.52 TB free** → `/zfs/hot/*` (no redundancy) |
| ZFS cold | `archive` 9.09 TB, **~8.49 TB free** → `/zfs/archive/*` (HDD) |

### Toolchain present / missing
- **Present:** `git`, `gh`, `python3`, `docker`, `clang`, `gcc`, `cc`, **Rust
  via rustup** (`~/.cargo`, `~/.rustup`), **CUDA 13.3 + nvcc**, Infisical
  (`~/.infisical`), HF cache (`~/.cache/huggingface`), `cmake`
  (`/home/croyse/calyx/bin/cmake`, 4.3.3), and `protoc`
  (`/home/croyse/calyx/bin/protoc`, libprotoc 35.0).
- **Missing:** none required for the current Calyx build/test/FSV gate. If a
  later phase needs another tool, install it inside `CALYX_HOME` and read it
  back before claiming it exists.

### Resident services to REUSE (loopback; do not start throwaways — PRD `16 §9`)
| Port | Service |
|---|---|
| `127.0.0.1:8088` | TEI general embedder (gte-multilingual-base, 768-d) |
| `127.0.0.1:8089` | TEI GTE reranker |
| `127.0.0.1:8090` | TEI legal ModernBERT (768-d) |
| `127.0.0.1:9090/9091/9093/9094` | Prometheus / pushgateway / Alertmanager |
| `*:8080` | existing app surface (leapable) — **do not touch** |

The TEI services use `infra/aiwonder/bin/tei-driver610-entrypoint.sh` as the
host-mounted entrypoint on driver-610 hosts. That wrapper parses both
`CUDA Version` and `CUDA UMD Version` from `nvidia-smi`, strips stale
`/usr/local/cuda/compat` from `LD_LIBRARY_PATH` on CUDA 13.3, and fails closed
if `text-embeddings-router` would still map the CUDA 12.9 compat `libcuda`.
Install or refresh the systemd drop-ins with
`sudo infra/aiwonder/bin/install-tei-driver610-runtime.sh --restart`.

### Existing projects on the box — **off-limits**
`leapable`/`leapable-build*`, `contextgraph` (home + `/zfs/hot/contextgraph`,
`/zfs/archive/contextgraph`), PostgreSQL (`/zfs/hot/postgres*`), Redis,
marketplace, seaweedfs, the `dist`/`leapable` user dirs. Calyx **reads none of
them, writes none of them, depends on none of them.** It may *reuse* the shared
read-only services (TEI lenses, Prometheus) and lift ContextGraph algorithm
*source as seeds* into its own crates — by copying into `CALYX_HOME`, never by
linking against the live project.

## 3. The PRD assumptions this readback corrects

| PRD said | Reality on aiwonder | Consequence for the plan |
|---|---|---|
| "No `rustc` on box → cross-build + ship binary" (`00`,`16`) | **Rust is installed (rustup).** | **Build natively on aiwonder.** No cross-build/`.deb` pipeline needed for dev. (Keep cross-build only if a future minimal-deploy target needs it.) |
| user `leapable`, paths `/opt/leapable/calyx`, `/zfs/hot/calyx` | current user is **`croyse`**; no `calyx` datasets exist; `/opt` needs root | Calyx home = `/home/croyse/calyx`; ZFS `calyx` datasets are an operator/sudo step (§4). |
| systemd unit `calyxd.service` runs as `leapable` | `croyse` has password-backed sudo available through the operator-provided local env var name; it is not passwordless. | Server/systemd/ZFS phases are **sudo-gated but agent-owned when authorized**: use password-backed sudo from `.env` without printing the value, then read back the service/dataset state. Dev/test normally does not need systemd. |

These are noted in the `[CONTEXT] Landmines` issue (PRD `29`).

## 4. The self-contained Calyx layout (binding)

**One root, nothing outside it.** `CALYX_HOME=/home/croyse/calyx`.

```
/home/croyse/calyx/                     # CALYX_HOME — the entire project
  repo/                                 # the git checkout (chrisroyse/calyx-dev)
    crates/  Cargo.toml  rust-toolchain.toml  …
  target/                               # CALYX_CARGO_TARGET_DIR (explicit build output)
  data/                                 # Aster vaults (interim; → ZFS hot when provisioned)
  datasets/                             # downloaded real datasets (interim; → ZFS cold)
  .hf-cache/                            # HF_HOME (models/lenses pulled here)
  logs/                                 # structured logs (rotated, bounded)
  tmp/                                  # scratch (staged in-place; cleaned each turn)
  bin/                                  # locally-installed userspace tools (cmake, protoc)
  .venv-cuvs/                           # RAPIDS cuVS/cu13 wheel stack for DiskANN GPU builds
  vendor/onnxruntime-v1.26.0/           # aiwonder-built ORT CUDA 13 dynamic library
  env.sh                               # sources ~/.cargo/env + exports CALYX_* + pinned CUDA paths
```

**Toolchain reuse, output isolation:** reuse the already-installed
`~/.cargo`/`~/.rustup` (don't duplicate a toolchain), but keep build outputs
inside `CALYX_HOME`. `repo/env.sh` exposes
`CALYX_CARGO_TARGET_DIR=/home/croyse/calyx/target` and clears foreign inherited
`CARGO_TARGET_DIR` values; verified builds set `CARGO_TARGET_DIR` explicitly via
`scripts/build-verified-calyx.sh`. It also exports
`CALYX_ORT_LIB_DIR=/home/croyse/calyx/vendor/onnxruntime-v1.26.0/build/Linux/Release`,
`ORT_DYLIB_PATH=$CALYX_ORT_LIB_DIR/libonnxruntime.so`, and prepends that
directory to `LD_LIBRARY_PATH`; the `ort/load-dynamic` crates intentionally
fail closed if those bytes are missing. It also discovers the aiwonder cuVS
wheel (`libcuvs-cu13==26.6.0`), CUDA 13.3 runtime libraries, RAFT/RMM support
libraries, and NVIDIA wheel libraries, exports their joined
`CALYX_ELF_RUNPATH`, and appends matching `-Wl,-rpath,...` flags to Rust builds
that source the entrypoint. `scripts/build-verified-calyx.sh` reads the
produced ELF dynamic section and runs `ldd` with `LD_LIBRARY_PATH` cleared, so a
release binary that would only run in a specially sourced shell fails the gate
instead. `HF_HOME=.../.hf-cache` keeps model/cache output inside `CALYX_HOME`.
`repo/env.sh` is the single entrypoint every session sources.

### Target ZFS datasets (preferred for hot/cold data; sudo-gated, one-time)
Matches PRD `04 §3` / `16 §3`. Create via authorized password-backed sudo
when the owning phase needs ZFS-backed Calyx storage:
```bash
sudo zfs create hotpool/calyx        -o mountpoint=/zfs/hot/calyx
sudo zfs create archive/calyx        -o mountpoint=/zfs/archive/calyx
sudo zfs create archive/calyx-restic -o mountpoint=/zfs/archive/calyx/restic
sudo chown -R croyse:croyse /zfs/hot/calyx /zfs/archive/calyx
```
PH61 T07 security FSV uses the encrypted child dataset
`hotpool/calyx/secure`, mounted through `/home/croyse/calyx/data/secure`;
live readback on 2026-06-13 showed `encryption=aes-256-gcm`, `keyformat=raw`,
and a root-only key file outside the repo. The older parent Calyx datasets may
remain `encryption=off`; do not cite them as the PH61 at-rest encryption proof.
Then `data/` and `datasets/` under `CALYX_HOME` are relocated/symlinked to
`/zfs/hot/calyx` (WAL, base CF, active slots, indexes, kernel/guard) and
`/zfs/archive/calyx` (raw f32 sidecars, retired slots, ledger archive, restic,
datasets). **Until provisioned**, Calyx runs entirely from `CALYX_HOME` on the
880 GB NVMe root — fully functional, just without ZFS snapshots/restic for
Calyx data. The plan does **not block** on the operator: PH00 uses the home
dir; a later task relocates to ZFS when datasets exist.

ZFS gotchas to honor (PRD `04 §3`): reference disks by `wwn-`/`eui-`; stage
temp files **inside the destination dataset** (avoid `EXDEV` on rename);
`hotpool` has no redundancy → durability = WAL + ZFS snapshots + restic; whole-
host loss is accepted posture.

## 5. Secrets on aiwonder

- **Infisical** is installed (`~/.infisical`). Calyx's only standing secret is
  the **HF token** (models + gated datasets), already mirrored in `../../.env`.
  Prefer `infisical run … -- <cmd>` so values stay in memory; or export
  `HF_TOKEN` from `repo/env.sh` (which reads it from a `0600` file outside the
  repo, never committed).
- **Discipline (binding):** never `echo`/`act_type` a secret value into a shell
  that logs it; never write a value into the repo/issue/PR/chat — names only
  (DOCTRINE §8c). `.env` on the Windows side is gitignored; on aiwonder, keep
  the token in `~/.config/calyx/secrets.env` (`0600`), sourced by `env.sh`.

## 6. Userspace installs
- **cmake:** installed at `/home/croyse/calyx/bin/cmake`; current readback:
  `cmake version 4.3.3`.
- **protoc:** installed at `/home/croyse/calyx/bin/protoc`; current readback:
  `libprotoc 35.0`.
- **cuVS:** installed in `/home/croyse/calyx/.venv-cuvs` as
  `libcuvs-cu13==26.6.0`. `env.sh` exports the wheel's cuVS/raft/rmm
  CMake package directories and shared-library paths when the venv exists; do
  not install RAPIDS packages into system Python.
- **If reinstallation is needed:** download the official static tarball or
  prebuilt release into `CALYX_HOME/bin`, then prepend `CALYX_HOME/bin` and
  `~/.local/bin` to PATH in `env.sh`.
- Anything else (e.g. extra Rust components, `cargo-fuzz`, `cargo-mutants`,
  `criterion` are dev-deps): `cargo install`/`rustup component add` — all
  userspace, no sudo.

## 7. Build / store / run / test — all here
- **Build:** on aiwonder (`source ~/.cargo/env && cargo build`), output in
  `CALYX_HOME/target`. GPU code compiles against CUDA 13.3 for sm_120.
- **Store:** Aster vaults + datasets under `CALYX_HOME` (→ ZFS when provisioned).
- **Run:** `calyxd`/`calyx` CLI + reuse TEI lenses on aiwonder's RTX 5090.
- **Test:** every test (synthetic mechanics + real-dataset intelligence) runs
  on aiwonder against persisted state; local runs are authoring only and never
  count as FSV (PRD `28 §5`).

## 8. One-paragraph summary
Calyx is built and lives **only** on aiwonder, under `/home/croyse/calyx`,
reachable as `croyse@aiwonder.mst.com` over the Cisco VPN, using the box's
already-installed Rust + CUDA 13.3 + RTX 5090 (sm_120) and its resident TEI
lenses, with all build output, data, datasets, and caches kept inside that one
root, dedicated ZFS datasets provisioned by a one-time authorized sudo step,
password-backed sudo available for gated host work when needed, and absolutely
no contact with the existing
leapable/contextgraph/PostgreSQL state on the same machine.
