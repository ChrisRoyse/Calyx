# 01 — aiwonder Environment & the Self-Contained Calyx Layout

Everything Calyx is built/stored/run/tested here. This doc is the **live,
verified** picture of the box (readback 2026-06-06) and the binding rule that
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
4. **Rust on PATH:** non-login shells don't have it — every remote build/test
   command must `source /home/croyse/.cargo/env` first (or use an absolute
   `~/.cargo/bin/cargo`).

## 2. Verified hardware & OS (live readback 2026-06-06)

| Resource | Value (confirmed) |
|---|---|
| Host / user | `aiwonder` / `croyse` |
| OS | Ubuntu **26.04 LTS**, kernel **7.0.0-15-generic**, systemd, UTC |
| CPU | 32 threads (Ryzen 9-class, 16c/32t) |
| RAM | **121 GiB** total, ~90 GiB available steady-state, **0 swap** |
| GPU | **RTX 5090**, Blackwell, **sm_120 (compute_cap 12.0)**, **32607 MiB**, driver **595.71.05** |
| CUDA toolkit | **13.2** installed (`/usr/local/cuda` → 13.2; also 13.1, 13.0). `nvcc` V13.2.78 works |
| Root FS | `/dev/nvme0n1p2` ext4, 1.8 TB, **~880 GB free** (holds `/home/croyse`) |
| ZFS hot | `hotpool` 1.81 TB, **~1.52 TB free** → `/zfs/hot/*` (no redundancy) |
| ZFS cold | `archive` 9.09 TB, **~8.49 TB free** → `/zfs/archive/*` (HDD) |

### Toolchain present / missing
- **Present:** `git`, `gh`, `python3`, `docker`, `clang`, `gcc`, `cc`, **Rust
  via rustup** (`~/.cargo`, `~/.rustup`), **CUDA 13.2 + nvcc**, Infisical
  (`~/.infisical`), HF cache (`~/.cache/huggingface`).
- **Missing (install in userspace — see §6):** `cmake`, `protoc`.

### Resident services to REUSE (loopback; do not start throwaways — PRD `16 §9`)
| Port | Service |
|---|---|
| `127.0.0.1:8088` | TEI general embedder (gte-multilingual-base, 768-d) |
| `127.0.0.1:8089` | TEI GTE reranker |
| `127.0.0.1:8090` | TEI legal ModernBERT (768-d) |
| `127.0.0.1:9090/9091/9093/9094` | Prometheus / pushgateway / Alertmanager |
| `*:8080` | existing app surface (leapable) — **do not touch** |

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
| systemd unit `calyxd.service` runs as `leapable` | **no passwordless sudo** for croyse | Server/systemd phases (S16) are **sudo-gated**: operator runs the unit install, or we defer to a user-level runner. Dev/test never needs systemd. |

These are noted in the `[CONTEXT] Landmines` issue (PRD `29`).

## 4. The self-contained Calyx layout (binding)

**One root, nothing outside it.** `CALYX_HOME=/home/croyse/calyx`.

```
/home/croyse/calyx/                     # CALYX_HOME — the entire project
  repo/                                 # the git checkout (chrisroyse/calyx)
    crates/  Cargo.toml  rust-toolchain.toml  …
  target/                               # CARGO_TARGET_DIR (build output stays here)
  data/                                 # Aster vaults (interim; → ZFS hot when provisioned)
  datasets/                             # downloaded real datasets (interim; → ZFS cold)
  .hf-cache/                            # HF_HOME (models/lenses pulled here)
  logs/                                 # structured logs (rotated, bounded)
  tmp/                                  # scratch (staged in-place; cleaned each turn)
  bin/                                  # locally-installed userspace tools (cmake, protoc)
  env.sh                               # sources ~/.cargo/env + exports CALYX_* + CUDA paths
```

**Toolchain reuse, output isolation:** reuse the already-installed
`~/.cargo`/`~/.rustup` (don't duplicate a toolchain), but set
`CARGO_TARGET_DIR=/home/croyse/calyx/target` and `HF_HOME=.../.hf-cache` so all
*output* is inside `CALYX_HOME`. `repo/env.sh` is the single entrypoint every
session sources.

### Target ZFS datasets (preferred for hot/cold data; sudo-gated, one-time)
Matches PRD `04 §3` / `16 §3`. Created by the operator (root) once:
```bash
sudo zfs create hotpool/calyx        -o mountpoint=/zfs/hot/calyx
sudo zfs create archive/calyx        -o mountpoint=/zfs/archive/calyx
sudo zfs create archive/calyx-restic -o mountpoint=/zfs/archive/calyx/restic
sudo chown -R croyse:croyse /zfs/hot/calyx /zfs/archive/calyx
```
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

## 6. Userspace installs (no sudo needed)
- **cmake:** `python3 -m pip install --user cmake` (puts `cmake` in
  `~/.local/bin`) or download the official static tarball into `CALYX_HOME/bin`.
- **protoc:** download the prebuilt `protoc-*-linux-x86_64.zip` release into
  `CALYX_HOME/bin` (only needed if a crate uses prost/tonic — we avoid where
  possible). Prepend `CALYX_HOME/bin` and `~/.local/bin` to PATH in `env.sh`.
- Anything else (e.g. extra Rust components, `cargo-fuzz`, `cargo-mutants`,
  `criterion` are dev-deps): `cargo install`/`rustup component add` — all
  userspace, no sudo.

## 7. Build / store / run / test — all here
- **Build:** on aiwonder (`source ~/.cargo/env && cargo build`), output in
  `CALYX_HOME/target`. GPU code compiles against CUDA 13.2 for sm_120.
- **Store:** Aster vaults + datasets under `CALYX_HOME` (→ ZFS when provisioned).
- **Run:** `calyxd`/`calyx` CLI + reuse TEI lenses on aiwonder's RTX 5090.
- **Test:** every test (synthetic mechanics + real-dataset intelligence) runs
  on aiwonder against persisted state; local runs are authoring only and never
  count as FSV (PRD `28 §5`).

## 8. One-paragraph summary
Calyx is built and lives **only** on aiwonder, under `/home/croyse/calyx`,
reachable as `croyse@aiwonder.mst.com` over the Cisco VPN, using the box's
already-installed Rust + CUDA 13.2 + RTX 5090 (sm_120) and its resident TEI
lenses, with all build output, data, datasets, and caches kept inside that one
root, dedicated ZFS datasets provisioned by a one-time operator sudo step, no
passwordless sudo for routine work, and absolutely no contact with the existing
leapable/contextgraph/PostgreSQL state on the same machine.
