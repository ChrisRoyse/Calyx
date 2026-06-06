# T-002 — Provision the self-contained Calyx home (+ ZFS datasets)

**Phase:** PH00 · **Dep:** T-001 · **Sudo:** ZFS step yes (operator); home dir no

## Objective
Create the **one root** all Calyx work lives under — `CALYX_HOME=/home/croyse/
calyx` — and (operator, optional, non-blocking) the dedicated ZFS datasets.
Nothing Calyx ever writes outside this root.

## Preconditions
- T-001 done (access confirmed).

## Steps
1. **Create the home tree** (no sudo) on aiwonder:
   ```bash
   mkdir -p /home/croyse/calyx/{repo,target,data,datasets,.hf-cache,logs,tmp,bin}
   ls -la /home/croyse/calyx
   ```
2. **(Operator / sudo, optional now — does NOT block dev)** create the ZFS
   datasets and chown to croyse:
   ```bash
   sudo zfs create hotpool/calyx        -o mountpoint=/zfs/hot/calyx
   sudo zfs create archive/calyx        -o mountpoint=/zfs/archive/calyx
   sudo zfs create archive/calyx-restic -o mountpoint=/zfs/archive/calyx/restic
   sudo chown -R croyse:croyse /zfs/hot/calyx /zfs/archive/calyx
   ```
   If/when created, point data + datasets at them (symlink, keeping the
   self-contained root):
   ```bash
   rmdir /home/croyse/calyx/data /home/croyse/calyx/datasets 2>/dev/null
   ln -s /zfs/hot/calyx      /home/croyse/calyx/data
   ln -s /zfs/archive/calyx  /home/croyse/calyx/datasets
   ```
   Until then, `data/` and `datasets/` are plain dirs on the NVMe root (880 GB
   free) — fully functional; relocate later.
3. Confirm nothing was created outside `CALYX_HOME` (and no existing project
   touched).

## Deliverables
- `CALYX_HOME` tree on aiwonder; (optional) ZFS datasets + symlinks.

## FSV gate
`ls -la /home/croyse/calyx` shows the full tree; if datasets were created,
`zfs list` shows `hotpool/calyx` + `archive/calyx` and `data`/`datasets`
resolve to them; `find / -newer … -path '!*/calyx/*'` (spot check) shows no
Calyx file outside the root; leapable/contextgraph/postgres untouched.

## Done
The self-contained root exists; the ZFS relocation is either done or recorded as
a pending operator step in the Landmines issue (does not block T-003+).

## Refs
`../01_AIWONDER_ENVIRONMENT.md §4`, PRD `04 §3`/`16 §3`, DOCTRINE §8c.
