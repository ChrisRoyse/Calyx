# T-004 — Install userspace build deps (cmake, protoc)

**Phase:** PH00 · **Dep:** T-002 · **Sudo:** no

## Objective
Provide the two missing native build tools (`cmake`, `protoc`) **in userspace**
(no sudo), inside `CALYX_HOME/bin` / `~/.local/bin`, so Rust crates with C/proto
build deps compile.

## Preconditions
- T-002 (home + `bin/`). Confirmed missing on aiwonder: `cmake`, `protoc`.
  Present: gcc/clang/cc.

## Steps
1. **cmake** (pip provides a userspace binary):
   ```bash
   python3 -m pip install --user cmake
   command -v cmake && cmake --version    # in ~/.local/bin
   ```
   (Alternative: download the official `cmake-*-linux-x86_64.tar.gz` and extract
   into `CALYX_HOME/bin`.)
2. **protoc** (only needed if a crate uses prost/tonic — we avoid where possible;
   install anyway to unblock):
   ```bash
   PV=27.3   # pick a current release
   curl -L -o /tmp/protoc.zip \
     https://github.com/protocolbuffers/protobuf/releases/download/v$PV/protoc-$PV-linux-x86_64.zip
   mkdir -p /home/croyse/calyx/bin/protoc && \
     (cd /home/croyse/calyx/bin/protoc && unzip -o /tmp/protoc.zip)
   ln -sf /home/croyse/calyx/bin/protoc/bin/protoc /home/croyse/calyx/bin/protoc-bin
   ```
   Ensure `CALYX_HOME/bin` + `~/.local/bin` are on PATH (done by `env.sh`).
3. Re-source `env.sh`; verify both tools resolve.

## Deliverables
- `cmake` + `protoc` available on PATH, entirely in userspace under `CALYX_HOME`/
  `~/.local`.

## FSV gate
`source env.sh && cmake --version && protoc --version` both succeed; binaries
resolve to paths inside `CALYX_HOME`/`~/.local` (verify with `command -v`),
proving no system-wide (sudo) install.

## Done
Both tools present in userspace; nothing installed system-wide.

## Refs
`../01_AIWONDER_ENVIRONMENT.md §6`, A34 (free), DOCTRINE §8c (no sudo).
