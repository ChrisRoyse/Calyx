# T-001 — Verify aiwonder access + record system baseline

**Phase:** PH00 · **Dep:** — · **Sudo:** no

## Objective
Confirm the agent can reach aiwonder and capture a dated, byte-level **system
baseline** (the FSV reference everything else is measured against). This is the
first FSV: read reality, don't assume the PRD.

## Preconditions
- Cisco AnyConnect VPN up (`vpn.pcrecruiter.net`, creds in `../../../.env`).
- `.env` filled (it is).

## Steps (from this Windows box; non-interactive SSH via askpass — see `.env`)
1. Reachability: `Test-NetConnection aiwonder.mst.com -Port 22` → `TcpTestSucceeded`.
2. SSH and capture baseline (base64 a script; `| base64 -d | bash`), recording:
   - `uname -a`, `/etc/os-release`
   - `nproc`, `free -h`
   - `nvidia-smi --query-gpu=name,driver_version,memory.total,compute_cap --format=csv`
   - `/usr/local/cuda/bin/nvcc --version`
   - `source ~/.cargo/env && rustc --version && cargo --version`
   - `zfs list -o name,used,avail,mountpoint`
   - `df -h | grep -vE 'tmpfs|loop'`
   - `ss -tlnp | grep -E ':808|:809|:9090'` (resident TEI + Prometheus)
3. Save the captured output to `repo/docs/implementation/baseline-<date>.md`
   once the repo exists (T-006); until then keep it in the session record.

## Deliverables
- A recorded, dated baseline of GPU/CPU/RAM/CUDA/Rust/ZFS/services.

## FSV gate
The baseline is **read from the box** (not the PRD): GPU = RTX 5090 sm_120 /
driver 595.71.05 / 32 GB; CUDA 13.2; Rust present via rustup; ZFS hot+cold
pools; TEI on :8088/:8089/:8090. Any drift from `../01_AIWONDER_ENVIRONMENT.md`
is noted in the Landmines issue (T-007).

## Done
Baseline captured and matches (or amends) the environment doc; SSH works
non-interactively.

## Refs
`../01_AIWONDER_ENVIRONMENT.md`, DOCTRINE §0/§8c, `../02_WORKING_AGREEMENT.md §2`.
