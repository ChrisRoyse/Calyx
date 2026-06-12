# 02 — Build Performance & Disk Hygiene

How Calyx builds fast and keeps the shared build host's disk bounded, and the
binding rule for **where each setting lives** so we never reintroduce the
machine-specific-config-in-git footgun (PR #662) or the runaway `target/` dir.

All builds happen on aiwonder (see `01_AIWONDER_ENVIRONMENT.md`): a Ryzen 9
9950X (16c/32t), 121 GiB RAM, RTX 5090, building a 18-crate Rust workspace into
a shared `CARGO_TARGET_DIR=/home/croyse/calyx/target` used by several agents.

---

## 1. The two problems we fixed (measured 2026-06-12)

| Symptom | Root cause | Evidence |
|---|---|---|
| `target/` was **196 GB** for a 23 MB repo | Cargo never GCs superseded artifacts; the default `debug = 2` made every dev/test executable **~280 MB**, and continuous multi-agent rebuilds piled up **11,672** files in `deps/` (all < 14 days old — pure churn, not age) plus a **61 GB** `incremental/` cache | `du -sh target/debug/{deps,incremental}` = 131 GB + 61 GB; biggest files were ~280 MB `calyx-<hash>` test binaries; rlibs were only 6.3 GB |
| Link phase under-using the 32-core box | Default linker is `rust-lld` (good, default since Rust 1.90) but not as parallel as `mold`, which was installed but unconfigured | `which mold` = `/usr/bin/mold 2.40.4`; no `~/.cargo/config.toml` |

## 2. The fix, and the binding rule for where it lives

There are two classes of setting and they live in two different places **on
purpose**:

### Portable → committed in the repo
These are machine-agnostic and benefit every checkout (Linux, Windows, CI), so
they belong in version control.

- **`Cargo.toml` `[profile.dev]` `debug = "line-tables-only"`** — keeps function
  names + `file:line` in panic/test backtraces (all the gate needs) while
  cutting executable size and link time several-fold.
- **`Cargo.toml` `[profile.dev.package."*"]` `debug = false`** — dependencies are
  never stepped into here; dropping their debuginfo is the single biggest
  `target/` size reduction with zero loss of first-party debuggability.
  (`profile.test` inherits `dev`, so `cargo test` binaries get this too.)
- **`scripts/check.sh` `export CARGO_INCREMENTAL=0`** — the gate is a one-shot
  build; incremental only adds overhead and disk churn there.

### Machine-specific → NOT committed (provisioned on the host)
A linker binary and a GC policy are properties of the *machine*, not the
project. Putting them in the repo's `.cargo/config.toml` would break Windows/CI
checkouts (an absolute mold path / a clang requirement they don't have) — the
exact class of bug PR #662 removed. They are provisioned by
**`scripts/aiwonder-build-setup.sh`**, which writes a Cargo config one level
*above* the repo, inside `CALYX_HOME`:

```
/home/croyse/calyx/.cargo/config.toml      # machine-local, gitignored by being outside repo/
  [build] incremental = false              # bound disk on this high-churn host
  [target.x86_64-unknown-linux-gnu]
    linker = "clang"
    rustflags = ["-C", "link-arg=-fuse-ld=mold"]
```

Cargo auto-discovers this for every build under `CALYX_HOME` (the repo and all
worktrees are children of it) **without** affecting other projects on the box
and **without** Windows/CI ever seeing it.

## 3. Keeping `target/` bounded forever

Cargo has no built-in GC, and on this box churn (not age) is the enemy — age-
based pruning reclaims ~0 because everything is days old. So we bound by **size**:

- **`scripts/target-gc.sh`** runs `cargo sweep --maxsize ${CALYX_TARGET_MAXSIZE:-40GB}`,
  removing the oldest artifacts until the dir is under the cap. Worst case a
  stale crate recompiles next build — never a correctness issue.
- **`scripts/aiwonder-build-setup.sh`** installs a **daily user cron** (04:00 UTC,
  no sudo) that runs it, logging to `CALYX_HOME/logs/target-gc.log`.
- The `incremental/` cache is always safe to delete manually for an instant
  reclaim: `rm -rf "$CARGO_TARGET_DIR"/debug/incremental`.

## 4. Provisioning a (re)built host

```bash
cd /home/croyse/calyx/repo
bash scripts/aiwonder-build-setup.sh        # writes machine-local config, installs cargo-sweep + cron
cargo build -v 2>&1 | grep -- '-fuse-ld=mold'   # verify mold is the linker
bash scripts/target-gc.sh                   # one manual GC pass (optional)
```

Everything is userspace and idempotent; re-run any time.

## 5. Why not also `target-cpu=native` / LTO / sccache?

- `target-cpu=native` and LTO help **runtime** performance but slow the dev/gate
  build and bust the cache; keep them for release/bench artifacts only, not the
  default profile that the gate uses.
- `sccache` was considered but adds a daemon + cache dir to manage; with mold +
  `line-tables-only` + bounded GC the build is already fast and the disk bounded,
  so it is intentionally **not** used to keep the toolchain simple. Revisit only
  if cold-cache dep builds become the bottleneck.
