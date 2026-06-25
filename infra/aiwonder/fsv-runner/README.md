# Reboot-resilient long-run supervisor (#888)

Long-running Calyx FSV/ingest jobs were launched as ad-hoc session processes
(see the original `#869` runner under `fsv/issue869-*/run_*.sh`). A reboot or a
lost login session silently stops such a run, and process liveness is not
represented as a durable unit with an explicit desired state, resume command,
and last-good cursor. This directory is the repo-owned, public-safe fix.

## What it provides

| File | Role |
|---|---|
| `calyx-longrun@.service` | systemd **user** template unit, one instance per run-id (`%i`). With `loginctl enable-linger`, the unit — and its job — return after a reboot. |
| `longrun-register.sh` | Writes the immutable `spec.json`, pins the executable's current sha256, installs the template, and `enable --now`s the instance. Refuses to run if linger is off (fail loud). |
| `longrun-supervise.sh` | `ExecStart` of the unit. Single-writer lock, stale-pidfile reclaim, exe-byte pinning re-check (#862), state-file heartbeat/cursor, already-complete no-op, and **idempotent resume** by re-invoking the (SoT-aware) job. |
| `fsv-synthetic-ingest.sh` | Real, idempotent, SoT-aware worker (SQLite `INSERT OR IGNORE`) used only to FSV the supervisor. |
| `fsv-longrun.sh` | Reproducible FSV: happy path + 4 edges, each printing the database state before/after. |

## Resume model (why it is correct)

The supervisor is **workload-agnostic**: it does not reimplement cursoring.
Correctness of resume is delegated to the wrapped job's own SoT-aware
idempotency, exactly as production already relies on it:

* `calyx ingest <vault> --batch <jsonl> --idempotent` skips rows already
  committed to the vault (content-hash idempotency).
* the FSV synthetic job uses `INSERT OR IGNORE` keyed by row id.

On resume the supervisor re-invokes the same command; the job continues from the
real committed cursor and never duplicates work. The supervisor's job is the
*durability and observability* a bare process lacks (unit, state file,
stale-pid reclaim, no-op on completion, pinned bytes).

## Register a real ingest run (e.g. the #869-class anchored re-ingest)

```bash
infra/aiwonder/fsv-runner/longrun-register.sh \
  --run-id anchored-reingest-2026XXXX \
  --exe /home/croyse/calyx/repo/target/release/calyx \
  --workdir /home/croyse/calyx/repo \
  --sot-kind calyx-vault \
  --sot-handle /home/croyse/calyx/vaults/<ULID> \
  --resume-command 'calyx ingest <vault> --batch <jsonl> --idempotent' \
  -- /home/croyse/calyx/repo/target/release/calyx ingest <vault> \
     --batch /zfs/.../medmcqa.anchored.jsonl --idempotent
```

Inspect / resume / observe:

```bash
systemctl --user status calyx-longrun@<run-id>.service
jq . /home/croyse/calyx/fsv/runs/<run-id>/state.json
systemctl --user restart calyx-longrun@<run-id>.service   # manual resume
```

> The active #869 run must NOT be interrupted to migrate it (per the issue).
> Adopt this for the next long run, or for #869 only if it stops again.

## FSV evidence

`fsv-longrun.sh` proves, against a real SQLite source of truth with a known
expected output (ids 1..N, no gaps/dupes):

* **happy path** — completes, exactly N rows, `status=completed`;
* **process restart without reboot** (`systemctl --user restart` mid-run) —
  resumes, reaches N, zero duplicate ids;
* **simulated reboot** (`stop` = power loss, `start` = boot) — resumes, reaches N;
* **stale pidfile / dead worker** — detected, reclaimed, run completes;
* **already-complete re-entry** — fast no-op, zero new rows.
