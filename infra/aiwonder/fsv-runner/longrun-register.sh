#!/usr/bin/env bash
# Register a reboot-resilient long-run job and start it under systemd (#888).
#
# Writes an immutable spec.json into a per-run directory, pins the executable's
# current sha256, installs the templated user unit instance, and starts it.
# With `loginctl enable-linger` (required, checked here) the unit — and thus the
# job — is brought back automatically after a reboot, where the supervisor's
# idempotent resume continues from the real SoT cursor.
#
# Usage:
#   longrun-register.sh --run-id <id> --exe <path> --workdir <dir> \
#       --sot-kind <sqlite|calyx-vault> --sot-handle <path-or-vault-dir> \
#       --resume-command <human-string> -- <command argv...>
set -euo pipefail

RUNS_ROOT="${CALYX_LONGRUN_ROOT:-/home/croyse/calyx/fsv/runs}"
UNIT_SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
UNIT_TEMPLATE="$UNIT_SRC_DIR/calyx-longrun@.service"

RUN_ID="" EXE="" WORKDIR="" SOT_KIND="" SOT_HANDLE="" RESUME_CMD=""
while [ $# -gt 0 ]; do
  case "$1" in
    --run-id) RUN_ID="$2"; shift 2;;
    --exe) EXE="$2"; shift 2;;
    --workdir) WORKDIR="$2"; shift 2;;
    --sot-kind) SOT_KIND="$2"; shift 2;;
    --sot-handle) SOT_HANDLE="$2"; shift 2;;
    --resume-command) RESUME_CMD="$2"; shift 2;;
    --) shift; break;;
    *) echo "CALYX_LONGRUN_USAGE: unknown arg $1" >&2; exit 2;;
  esac
done
ARGV=("$@")

err() { echo "CALYX_LONGRUN_REGISTER_FAILED: $*" >&2; exit 1; }
[ -n "$RUN_ID" ] || err "--run-id required"
[ -n "$EXE" ] && [ -x "$EXE" ] || err "--exe must be an executable path (got: $EXE)"
[ -n "$WORKDIR" ] && [ -d "$WORKDIR" ] || err "--workdir must be a directory"
[ -n "$SOT_KIND" ] || err "--sot-kind required"
[ -n "$SOT_HANDLE" ] || err "--sot-handle required"
[ -n "$RESUME_CMD" ] || err "--resume-command required"
[ "${#ARGV[@]}" -ge 1 ] || err "a command argv is required after --"
[ -f "$UNIT_TEMPLATE" ] || err "unit template not found: $UNIT_TEMPLATE"

# Linger is what makes a *user* unit reboot-resilient. Refuse to pretend
# otherwise — fail loud if it is off.
if ! loginctl show-user "$(id -un)" 2>/dev/null | grep -q '^Linger=yes'; then
  err "linger is OFF for $(id -un); run: loginctl enable-linger $(id -un)"
fi

RUN_DIR="$RUNS_ROOT/$RUN_ID"
mkdir -p "$RUN_DIR"
SPEC="$RUN_DIR/spec.json"
[ -f "$SPEC" ] && err "spec already exists ($SPEC) — run-id must be unique; resume the existing run instead"

EXE_SHA="$(sha256sum "$EXE" | awk '{print $1}')"
CMD_JSON="$(printf '%s\n' "${ARGV[@]}" | jq -R . | jq -s .)"
jq -n --arg id "$RUN_ID" --arg exe "$EXE" --arg sha "$EXE_SHA" --arg wd "$WORKDIR" \
      --arg sk "$SOT_KIND" --arg sh "$SOT_HANDLE" --arg rc "$RESUME_CMD" \
      --argjson cmd "$CMD_JSON" \
   '{run_id:$id, exe:$exe, exe_sha256:$sha, workdir:$wd,
     sot:{kind:$sk, handle:$sh}, command:$cmd, resume_command:$rc}' >"$SPEC"
echo "wrote $SPEC"

# Install the templated unit (idempotent) and start this instance.
mkdir -p "$HOME/.config/systemd/user"
install -m 0644 "$UNIT_TEMPLATE" "$HOME/.config/systemd/user/calyx-longrun@.service"
systemctl --user daemon-reload
systemctl --user enable --now "calyx-longrun@${RUN_ID}.service"
echo "started calyx-longrun@${RUN_ID}.service (spec=$SPEC)"
