# `calyx-gatebrokerd` — exact gate ownership boundary (#1455 / #1457)

Status: implementation contract. The installed daemon and client are
`/usr/libexec/calyx-gatebrokerd` and `/usr/bin/calyx-gatectl`; the release
barrier is `/usr/libexec/calyx-gate-stage-shim`. Deployment is repo-owned under
`infra/aiwonder/gatebroker/` and operator-installed as root.

## Problem and threat model

The #1421 gate scripts improved cooperative cleanup, but two hostile-same-UID
claims remain impossible in-process:

- a pathname identity check followed by `rmdir`, `unlink`, `cgroup.kill`, or a
  unit-name action can target a replacement installed after the check (#1455);
- a stage running as the controller UID can ask that UID's delegated user
  manager to create a sibling unit outside the gate cgroup (#1457).

The attacker controls stage argv, descendants, signals, directory entries
inside published scratch, and timing. It does not control root, PID 1, the
broker binary/config/journal, the system cgroup hierarchy, or broker-private
roots. Root compromise is outside this boundary.

There is no legacy fallback. Missing privileges, unsupported file handles,
cross-mount roots, journal corruption, manager mismatch, or incomplete cleanup
return a stable `CALYX_GATEBROKER_*` error and containment status 125.

## Identities and privilege topology

| Identity | Authority |
|---|---|
| `croyse` in `calyx-gate-clients` | Opens a gate lease through the control socket; cannot write shared containers or private roots. |
| root `calyx-gatebrokerd` | Owns the journal, namespace transactions, system-manager calls, pidfds, and held cgroup FDs. |
| locked `calyx-gate-worker` | Runs one serialized gate payload; is not a broker client and has no login, linger, user manager, or delegated cgroup. |

The first implementation deliberately permits one active run. A fixed worker
UID plus concurrent runs would recreate a same-UID cross-run boundary. Restore
concurrency only with separately provisioned per-run UIDs and equivalent FSV.

The broker is socket activated with `AF_UNIX/SOCK_SEQPACKET` at
`/run/calyx-gatebrokerd/control.sock`. The socket is
`root:calyx-gate-clients` mode 0660. Each connection is authenticated with
`SO_PEERCRED`; `BeginRun` immediately opens an exact owner pidfd. Worker
processes receive no global socket or controller FD.

## Filesystem roots

Mutable deletion authority and read-only execution authority are deliberately
different capabilities. A managed-root alias may create, quarantine, and
delete only registered children. An execution-root alias may only resolve a
descriptor-relative cwd for a stage; it can never be passed to create or
delete RPCs.

The checked-in aiwonder managed-root aliases are:

| Alias | Shared container | Private quarantine |
|---|---|---|
| `tmp` | `/var/lib/calyx-gatebrokerd/objects/tmp` | `/var/lib/calyx-gatebrokerd/private/quarantine/tmp` |
| `target` | `/var/lib/calyx-gatebrokerd/objects/target` | `/var/lib/calyx-gatebrokerd/private/quarantine/target` |

`/var/lib/calyx-gatebrokerd` is the root:root 0711 state anchor. Every path
component from `/var` through that anchor is a real root-owned directory with
no group/other write bit or authority-changing ACL, so the controller cannot
rename or replace the anchor. Shared containers are root:root 0711: stages can
traverse a published worker-owned child but cannot create, rename, or remove
top-level names. `/var/lib/calyx-gatebrokerd/private` and its quarantine
children are root:root 0700. The shared and private side of each alias must
resolve beneath the state anchor, with private sides strictly beneath the
root-only private directory, and must be on the same mount. Comparing `st_dev`
alone is insufficient; installation also compares the mount point and performs
a real two-way `rename(2)` probe.

The `source` execution root is `/home/croyse/calyx`, expected to be
`croyse` mode 0755. It remains caller-owned and read-only to broker-launched
stages; installation never creates, re-owns, or chmods it. Startup opens this
root once, verifies its configured owner/mode and symlink-free path, and cwd
resolution must use `openat2(2)` relative to that descriptor with
`RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS`. The resolved cwd
descriptor, rather than a re-opened absolute pathname, is the execution
authority. Failure of any requested resolution guarantee is fatal; there is no
`realpath(3)` or string-prefix fallback.

Startup must additionally create a private probe, prove
`renameat2(RENAME_NOREPLACE)`, capture it with `name_to_handle_at`, move it, and
reopen it with `open_by_handle_at`. Unsupported filesystem operations make the
daemon unready. Device/inode values are diagnostics, never authorization.

The journal is
`/var/lib/calyx-gatebrokerd/private/journal/journal.sqlite`, root:root 0600
beneath a dedicated root:root 0700 journal directory. SQLite uses WAL,
`synchronous=FULL`, foreign keys, disabled trusted schema, and startup
integrity checking. A success response is not emitted until the journal commit
and every required file/directory `fsync` has completed.

## RPC contract

Frames are atomic, versioned, and at most 65,536 bytes. Unknown fields,
oversized argv/environment, NUL, traversal, raw UIDs, raw unit/cgroup names,
and arbitrary delete paths are rejected before state changes. Mutations carry
a UUID request ID and request hash: an exact retry returns its recorded result;
reuse with different bytes is an error.

The minimal operations are:

- `Health` / `Capabilities` — separate live feature and root readback;
- `BeginRun(profile)` — bind a run ID to peer credentials, pidfd, connection,
  and the single active-run lease;
- `CreateObject(run, root_alias, role, requested_leaf?)` — create privately,
  register provenance, and publish;
- `ExecStage(run, execution_root+relative, argv[], env[])` — resolve cwd through
  the configured read-only execution-root descriptor and start a system
  transient service from an argv vector, never a shell command string;
- `DeleteObject(run, object_id)` — exact quarantine, validate, and delete;
- `FinishRun(run, intended_status)` / `AbortRun(run, signal)` — drain process
  state before scratch and return the final containment status;
- `Inspect(run|object|operation)` — independently compare journal rows with the
  public/private namespace, systemd unit, and cgroup.

Every error records code, run/operation/object IDs, transition, syscall and
errno, and remediation in SQLite and journald. Environment values and secret
argv are not logged; their bounded hashes are.

## Durable directory state machine

Creation ordering is strict:

1. Commit `INTENT` before namespace mutation.
2. Create in private quarantine, capture an opaque handle, set final policy,
   fsync the object and parent, then commit `PREPARED`.
3. Publish with descriptor-relative `RENAME_NOREPLACE`, fsync both parents,
   then commit `PUBLISHED`.
4. Commit `COMMITTED`, then reply.

Deletion extends the journal with `DELETE_INTENT`, `QUARANTINED`, and
`DELETED`. It atomically moves the shared basename into private quarantine
before comparing the stored opaque handle. A match is recursively removed
through directory FDs only after the run cgroup is proven empty.

On mismatch, the broker restores with `RENAME_NOREPLACE`. If another entry has
already filled the shared name, both entries are preserved: the newly shared
entry remains and the moved entry stays in a root-only recovery slot. The
operation becomes `RECOVERY_REQUIRED` and returns 125. Linux has no
unlink-by-handle primitive, so an original parked under an unknown attacker
name may leak; it is never guessed or deleted.

### Restart replay

| Durable state | Required replay |
|---|---|
| `INTENT` | Prove no entry, then abort. Any entry is corruption. |
| `PREPARED` | Remove a matching private object. If publication occurred before the state commit, quarantine and validate the public object. |
| `PUBLISHED` | No success was reportable; roll back through exact quarantine. |
| `COMMITTED` | The control lease was lost. Prove the bound system unit/cgroup dead, delete registered scratch, and abort rather than resume. |
| `DELETE_INTENT` | Quarantine a matching public object and continue. |
| `QUARANTINED` | Validate and finish private deletion. |
| `DELETED` | Prove public/private absence and that the stored handle is stale. |

Both names present, a handle mismatch, a missing name with a live handle,
journal integrity failure, or unit/cgroup identity mismatch leaves readiness
red. Recovery never invents a terminal success.

## Stage containment

The broker calls the system manager's typed `StartTransientUnit` API directly.
There is no stopped launcher before handoff. Each broker-chosen random unit is
a system service under a random per-run system slice and is bound to
`calyx-gatebrokerd.service`. It runs as `calyx-gate-worker` with `Type=exec`,
`ExitType=cgroup`, `KillMode=control-group`, no capabilities,
`NoNewPrivileges`, `Delegate=no`, and read-only cgroup visibility. Profiles
explicitly grant only required repo/toolchain, scratch, loopback, and GPU
access; missing access is an error, not a broader profile fallback.

The broker holds a systemd unit reference and records its `InvocationID`.
After the manager publishes `ControlGroup`, it opens that path beneath an open
cgroup-v2 root with `openat2` and retains the directory FD. Population and
`cgroup.kill` are accessed relative to that FD, never through a checked-then-
reopened pathname. Unit-name actions stop after the held unit reference is
released.

Owner pidfd readiness, control disconnect, broker shutdown, or an explicit
abort stops the referenced unit, kills the held run-slice cgroup if needed,
and proves `cgroup.events populated=0`. `BindsTo` drains work when the broker
process crashes; restart replay independently verifies the result.

## Installation and verification

Before first install, drain all legacy gates. The installer leaves
`/home/croyse/calyx` caller-owned and verifies it as the configured read-only
execution root. It creates managed roots only beneath the root-owned
`/var/lib/calyx-gatebrokerd` anchor. It refuses to chown or chmod an existing
path: an old mode-0700 state directory or any mismatched owner/mode requires an
explicit, offline operator migration. Absent managed roots are created by the
installed tmpfiles policy; existing roots must already match it.

```bash
sudo install -o root -g root -m 0755 target/release/calyx-gatebrokerd \
  /usr/libexec/calyx-gatebrokerd
sudo install -o root -g root -m 0755 target/release/calyx-gate-stage-shim \
  /usr/libexec/calyx-gate-stage-shim
sudo install -o root -g root -m 0755 target/release/calyx-gatectl \
  /usr/bin/calyx-gatectl
sudo bash infra/aiwonder/gatebroker/install-gatebroker.sh --install
sudo bash infra/aiwonder/gatebroker/install-gatebroker.sh --verify-only
```

An existing `/etc/calyx-gatebrokerd/config.toml` is never overwritten. Unit
byte changes while the broker is active are refused; finish all runs and stop
the service before applying them. The installer never removes journal or
quarantine state.

## Full-state verification on aiwonder

The source of truth is the SQLite transition row plus the real public/private
namespaces and the systemd/cgroup state. For every scenario print those values
before, immediately after the action, and after a separate read.

Minimum happy-path data is one run, one directory, one small known sentinel,
and one short process. Required manual cases are:

1. Create, execute a stage that writes the sentinel, inspect `COMMITTED`, then
   finish and independently prove `DELETED`, both names absent, unit absent,
   cgroup empty, and PID gone.
2. Park the original and install a different shared-name sentinel. Deletion
   must return 125; both known SHA-256 values remain, no replacement is
   deleted, and the row is `RECOVERY_REQUIRED`.
3. SIGKILL the daemon after each of `INTENT`, `PREPARED`, `PUBLISHED`,
   `DELETE_INTENT`, and `QUARANTINED`; let systemd restart it and read the
   replayed journal and namespaces.
4. Run `systemd-run --user`, an explicit controller-user bus request,
   `setsid`, and a double fork from a real worker stage. No sibling unit may
   exist and no process may appear outside the held run slice.
5. SIGKILL the controller before manager handoff and after handoff. Then
   SIGKILL the broker with a live stage. Each case must prove exact unit,
   cgroup, PID, journal, and scratch outcomes independently.
6. Exercise invalid/empty/maximum RPC frames, a cross-mount root, unsupported
   file handles, unsafe modes, ENOSPC, and journal corruption. State must not
   advance and no legacy helper may run.

Tests use the real system manager, cgroup v2, production filesystem, worker
UID, socket, and SQLite journal. A fake `systemd-run`, mock filesystem, or a
return-value-only assertion cannot close #1455 or #1457.
