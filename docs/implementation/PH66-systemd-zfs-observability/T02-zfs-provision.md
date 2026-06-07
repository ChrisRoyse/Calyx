# PH66 · T02 — ZFS dataset provisioning + data relocation

| Field | Value |
|---|---|
| **Phase** | PH66 — systemd + ZFS provisioning + Prometheus/Grafana |
| **Stage** | S16 — Server & Deployment |
| **Crate** | `infra` (no Rust crate) |
| **Files** | `infra/aiwonder/ops/provision-zfs.sh`, `infra/aiwonder/ops/relocate-data.sh` |
| **Depends on** | T01 (service running so vault can be stopped cleanly for relocation) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/16 §3`, `01 §4` |

## Goal

Write the idempotent operator script that creates the three ZFS datasets
(`hotpool/calyx` → `/zfs/hot/calyx`, `archive/calyx` → `/zfs/archive/calyx`,
`archive/calyx-restic` → `/zfs/archive/calyx/restic`) and sets ownership to
`croyse:croyse`. Then write the no-sudo relocation script that `rsync`s
`CALYX_HOME/data/` into `/zfs/hot/calyx/` and updates `calyx.toml`'s
`vault_path`. The relocation script stages temp files inside the destination
dataset (never across dataset boundaries) to avoid `EXDEV` errors. Pools are
referenced by name, not device path; the scripts document the `wwn-`/`eui-`
requirement for disk-level operations. Single-host no-HA posture is stated
explicitly in the script header.

> **[OPERATOR] steps:** `provision-zfs.sh` requires sudo for `zfs create` and
> `chown`. `relocate-data.sh` does not require sudo (it operates on paths already
> owned by `croyse`). Both scripts are idempotent.

## Build (checklist of concrete, code-level steps)

- [ ] `infra/aiwonder/ops/provision-zfs.sh` — operator script:
  ```bash
  #!/usr/bin/env bash
  # [OPERATOR] Requires sudo. Idempotent: skips datasets that already exist.
  # POSTURE: single-host, no off-machine replica. Durability = WAL + ZFS snapshots
  #          + restic to archive/calyx-restic. Whole-host loss is accepted posture.
  # Disk reference: reference disks by wwn-/eui- for stable IDs across reboots.
  #                 Pools (hotpool, archive) already abstract the device IDs.
  set -euo pipefail
  create_if_absent() {
    local ds="$1" mp="$2"
    if zfs list "$ds" &>/dev/null; then
      echo "Dataset $ds already exists — skipping"
    else
      sudo zfs create "$ds" -o mountpoint="$mp"
      echo "Created $ds → $mp"
    fi
  }
  create_if_absent hotpool/calyx        /zfs/hot/calyx
  create_if_absent archive/calyx        /zfs/archive/calyx
  create_if_absent archive/calyx-restic /zfs/archive/calyx/restic
  sudo chown -R croyse:croyse /zfs/hot/calyx /zfs/archive/calyx
  echo "ZFS provisioning complete"
  zfs list hotpool/calyx archive/calyx archive/calyx-restic
  ```
- [ ] `infra/aiwonder/ops/relocate-data.sh` — no-sudo relocation script:
  ```bash
  #!/usr/bin/env bash
  # Relocates CALYX_HOME/data/ → /zfs/hot/calyx/  (no sudo required)
  # Stages temp inside /zfs/hot/calyx/ to avoid EXDEV cross-dataset renames.
  # Updates infra/aiwonder/calyx.toml vault_path after successful rsync.
  set -euo pipefail
  SRC="${CALYX_HOME:-/home/croyse/calyx}/data/"
  DST="/zfs/hot/calyx/"
  TOML="${CALYX_HOME:-/home/croyse/calyx}/repo/infra/aiwonder/calyx.toml"

  [[ -d "$DST" ]] || { echo "ERROR: $DST not mounted (run provision-zfs.sh first)"; exit 1; }
  [[ -d "$SRC" ]] || { echo "ERROR: $SRC not found"; exit 1; }

  # rsync with --checksum; temp dir inside DST to avoid EXDEV
  TMPDIR="$DST/.rsync-tmp"
  mkdir -p "$TMPDIR"
  rsync -av --checksum --temp-dir="$TMPDIR" "$SRC" "$DST"
  rm -rf "$TMPDIR"

  # Update calyx.toml vault_path
  sed -i "s|^vault_path = .*|vault_path = \"/zfs/hot/calyx\"|" "$TOML"
  echo "Relocation complete. vault_path updated in calyx.toml"
  echo "Verify: ls -la $DST"
  ls -la "$DST"
  ```
- [ ] Both scripts have a header comment stating the single-host posture and the
  no-HA disclaimer: "No off-machine replica; whole-host loss is accepted posture
  for this deployment (`16 §3`)."
- [ ] `infra/aiwonder/calyx.toml` `[storage]` section: `vault_path`, `wal_path`,
  `log_path` all pointing to `/zfs/hot/calyx/...` post-relocation values
  documented with pre-provisioning fallback comment

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] shell lint: `bash -n infra/aiwonder/ops/provision-zfs.sh` exits 0
- [ ] shell lint: `bash -n infra/aiwonder/ops/relocate-data.sh` exits 0
- [ ] unit: `provision-zfs.sh` with `zfs list` returning "dataset exists" on
  both datasets → script prints "already exists — skipping" for each, exits 0
  (mock `zfs` with a stub in test; assert no `sudo zfs create` called a second
  time)
- [ ] unit: `relocate-data.sh` with `$DST` not mounted → exits 1 with
  `"ERROR: … not mounted"` message
- [ ] edge: `rsync` temp dir is inside `$DST` (assert `--temp-dir` flag present
  in the rsync invocation — grep the script source)
- [ ] edge: re-run `relocate-data.sh` when data is already relocated → rsync
  is a no-op (checksums match), `calyx.toml` is not double-updated (sed is
  idempotent when value is already correct)
- [ ] fail-closed: `EXDEV` cannot occur because temp dir is inside destination
  dataset — assert this by grep of `--temp-dir="$TMPDIR"` where `$TMPDIR` is
  under `$DST`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `zfs list hotpool/calyx archive/calyx archive/calyx-restic`; `ls
  -la /zfs/hot/calyx/`
- **Readback (operator runs `provision-zfs.sh`, then user runs
  `relocate-data.sh`):**
  ```bash
  # [OPERATOR]:
  bash infra/aiwonder/ops/provision-zfs.sh
  zfs list hotpool/calyx archive/calyx archive/calyx-restic
  # Must show all three with correct mountpoints

  # [USER — no sudo]:
  bash infra/aiwonder/ops/relocate-data.sh
  ls -la /zfs/hot/calyx/
  # Must show WAL files and vault data physically present
  grep vault_path infra/aiwonder/calyx.toml
  # Must show: vault_path = "/zfs/hot/calyx"
  ```
- **Prove:** `zfs list` output shows all three datasets with expected mountpoints;
  `ls -la /zfs/hot/calyx/` shows non-empty WAL/vault files; `vault_path` in
  `calyx.toml` is `/zfs/hot/calyx`. All outputs attached to PH66 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH66 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
