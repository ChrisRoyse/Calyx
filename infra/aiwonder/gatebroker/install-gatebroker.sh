#!/usr/bin/env bash
# Install or independently verify the repo-owned aiwonder gate-broker wiring.
# This script never weakens root policy, adopts an existing path, deletes an
# object, or falls back to the legacy same-UID helpers.
set -Eeuo pipefail
umask 077

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
mode=install
client_user="${CALYX_GATE_CLIENT_USER:-croyse}"

service_src="$script_dir/calyx-gatebrokerd.service"
socket_src="$script_dir/calyx-gatebrokerd.socket"
sysusers_src="$script_dir/calyx-gatebrokerd.sysusers.conf"
tmpfiles_src="$script_dir/calyx-gatebrokerd.tmpfiles.conf"
config_src="$script_dir/config.toml.example"

service_dst=/etc/systemd/system/calyx-gatebrokerd.service
socket_dst=/etc/systemd/system/calyx-gatebrokerd.socket
sysusers_dst=/usr/lib/sysusers.d/calyx-gatebrokerd.conf
tmpfiles_dst=/usr/lib/tmpfiles.d/calyx-gatebrokerd.conf
config_dir=/etc/calyx-gatebrokerd
config_dst="$config_dir/config.toml"
daemon_bin=/usr/libexec/calyx-gatebrokerd
shim_bin=/usr/libexec/calyx-gate-stage-shim
client_bin=/usr/bin/calyx-gatectl
socket_path=/run/calyx-gatebrokerd/control.sock
journal_path=/var/lib/calyx-gatebrokerd/private/journal/journal.sqlite
state_anchor=/var/lib/calyx-gatebrokerd
state_private="$state_anchor/private"
execution_source=/home/croyse/calyx

shared_tmp="$state_anchor/objects/tmp"
private_tmp="$state_private/quarantine/tmp"
shared_target="$state_anchor/objects/target"
private_target="$state_private/quarantine/target"

usage() {
  echo "usage: sudo bash infra/aiwonder/gatebroker/install-gatebroker.sh [--install|--verify-only]" >&2
}

fail() {
  local code="$1"
  shift
  printf '%s: %s\n' "$code" "$*" >&2
  exit 125
}

on_error() {
  local status="$?" line="$1" command="$2"
  trap - ERR
  printf 'CALYX_GATEBROKER_INSTALL_COMMAND_FAILED: line=%s status=%s command=%q\n' \
    "$line" "$status" "$command" >&2
  exit 125
}

trap 'on_error "$LINENO" "$BASH_COMMAND"' ERR

while [[ $# -gt 0 ]]; do
  case "$1" in
    --install) mode=install ;;
    --verify-only) mode=verify ;;
    -h|--help) usage; exit 0 ;;
    *) usage; exit 64 ;;
  esac
  shift
done

[[ "$EUID" -eq 0 ]] || fail CALYX_GATEBROKER_INSTALL_REQUIRES_ROOT \
  "run this exact script through sudo"

for command_name in cmp getcap getent grep id install mkdir python3 \
  readlink realpath rmdir sleep stat systemctl systemd-analyze systemd-sysusers \
  systemd-tmpfiles tr; do
  command -v "$command_name" >/dev/null 2>&1 || \
    fail CALYX_GATEBROKER_INSTALL_MISSING_COMMAND "$command_name is required"
done
if [[ "$mode" == install ]]; then
  command -v usermod >/dev/null 2>&1 || \
    fail CALYX_GATEBROKER_INSTALL_MISSING_COMMAND "usermod is required"
fi

assert_source() {
  local path="$1"
  [[ -f "$path" && ! -L "$path" ]] || \
    fail CALYX_GATEBROKER_INSTALL_SOURCE_INVALID "missing or linked source: $path"
}

assert_no_extended_acl() {
  local path="$1" label="$2" acl_names
  if ! acl_names="$(python3 - "$path" <<'PY'
import os
import sys

path = sys.argv[1]
try:
    names = os.listxattr(path, follow_symlinks=False)
except OSError as error:
    print(f"cannot inspect ACL xattrs for {path}: {error}", file=sys.stderr)
    raise SystemExit(1)
acl_names = sorted(
    name for name in names
    if name in {"system.posix_acl_access", "system.posix_acl_default"}
)
print(",".join(acl_names), end="")
PY
  )"; then
    fail CALYX_GATEBROKER_INSTALL_ACL_READ_FAILED "cannot read ACL for $label: $path"
  fi
  [[ -z "$acl_names" ]] || \
    fail CALYX_GATEBROKER_INSTALL_EXTENDED_ACL \
      "$label has authority-changing ACL xattrs at $path: $acl_names"
}

assert_regular_policy() {
  local path="$1" expected_mode="$2" label="$3" actual
  [[ -f "$path" && ! -L "$path" ]] || \
    fail CALYX_GATEBROKER_INSTALL_FILE_INVALID "$label is missing, linked, or not regular: $path"
  actual="$(stat -Lc '%u:%g:%a' -- "$path")" || \
    fail CALYX_GATEBROKER_INSTALL_STAT_FAILED "cannot stat $label: $path"
  [[ "$actual" == "0:0:$expected_mode" ]] || \
    fail CALYX_GATEBROKER_INSTALL_FILE_POLICY \
      "$label must be root:root mode $expected_mode: path=$path actual=$actual"
  assert_no_extended_acl "$path" "$label"
}

assert_dir_policy() {
  local path="$1" expected_mode="$2" label="$3" actual resolved
  [[ -d "$path" && ! -L "$path" ]] || \
    fail CALYX_GATEBROKER_INSTALL_DIR_INVALID "$label is missing, linked, or not a directory: $path"
  resolved="$(realpath -e -- "$path")" || \
    fail CALYX_GATEBROKER_INSTALL_REALPATH_FAILED "cannot resolve $label: $path"
  [[ "$resolved" == "$path" ]] || \
    fail CALYX_GATEBROKER_INSTALL_PATH_ALIAS "$label is not canonical: expected=$path actual=$resolved"
  actual="$(stat -Lc '%u:%g:%a' -- "$path")" || \
    fail CALYX_GATEBROKER_INSTALL_STAT_FAILED "cannot stat $label: $path"
  [[ "$actual" == "0:0:$expected_mode" ]] || \
    fail CALYX_GATEBROKER_INSTALL_DIR_POLICY \
      "$label must be root:root mode $expected_mode: path=$path actual=$actual"
  assert_no_extended_acl "$path" "$label"
}

assert_optional_dir_policy() {
  local path="$1" expected_mode="$2" label="$3"
  [[ ! -e "$path" && ! -L "$path" ]] || assert_dir_policy "$path" "$expected_mode" "$label"
}

assert_root_owned_path_chain() {
  local path="$1" label="$2"
  python3 - "$path" "$label" <<'PY' || \
    fail CALYX_GATEBROKER_INSTALL_ANCHOR_CHAIN \
      "$label must have a root-owned, non-writable, symlink-free path chain: $path"
import os
import stat
import sys

path, label = sys.argv[1:]
current = "/"
components = [part for part in path.split("/") if part]
for component in components:
    current = os.path.join(current, component)
    metadata = os.lstat(current)
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        raise SystemExit(f"{label}: non-directory or symlink in path chain: {current}")
    if metadata.st_uid != 0 or stat.S_IMODE(metadata.st_mode) & 0o022:
        raise SystemExit(
            f"{label}: path component permits rename authority: {current} "
            f"uid={metadata.st_uid} mode={stat.S_IMODE(metadata.st_mode):04o}"
        )
    acl_names = {
        name for name in os.listxattr(current, follow_symlinks=False)
        if name in {"system.posix_acl_access", "system.posix_acl_default"}
    }
    if acl_names:
        raise SystemExit(f"{label}: authority-changing ACL on {current}: {sorted(acl_names)}")
PY
}

assert_execution_root_policy() {
  local path="$1" expected_owner="$2" expected_mode="$3" label="$4" actual
  [[ -d "$path" && ! -L "$path" ]] || \
    fail CALYX_GATEBROKER_INSTALL_EXEC_ROOT_INVALID \
      "$label is missing, linked, or not a directory: $path"
  [[ "$(realpath -e -- "$path")" == "$path" ]] || \
    fail CALYX_GATEBROKER_INSTALL_EXEC_ROOT_ALIAS \
      "$label is not a canonical absolute path: $path"
  python3 - "$path" <<'PY' || \
    fail CALYX_GATEBROKER_INSTALL_EXEC_ROOT_SYMLINK \
      "$label contains an intermediate symlink: $path"
import os
import stat
import sys

current = "/"
for component in (part for part in sys.argv[1].split("/") if part):
    current = os.path.join(current, component)
    if stat.S_ISLNK(os.lstat(current).st_mode):
        raise SystemExit(f"symlink component: {current}")
PY
  actual="$(stat -Lc '%U:%a' -- "$path")" || \
    fail CALYX_GATEBROKER_INSTALL_STAT_FAILED "cannot stat $label: $path"
  [[ "$actual" == "$expected_owner:$expected_mode" ]] || \
    fail CALYX_GATEBROKER_INSTALL_EXEC_ROOT_POLICY \
      "$label must match configured owner/mode: path=$path expected=$expected_owner:$expected_mode actual=$actual"
  assert_no_extended_acl "$path" "$label"
}

assert_binary() {
  local path="$1" label="$2" policy capabilities
  [[ -f "$path" && ! -L "$path" && -x "$path" ]] || \
    fail CALYX_GATEBROKER_INSTALL_BINARY_INVALID "$label is not an executable regular file: $path"
  policy="$(stat -Lc '%u:%g:%a' -- "$path")"
  [[ "$policy" == "0:0:755" ]] || \
    fail CALYX_GATEBROKER_INSTALL_BINARY_POLICY \
      "$label must be root:root mode 0755 with no set-ID bits: path=$path actual=$policy"
  assert_no_extended_acl "$path" "$label"
  capabilities="$(getcap -n "$path")" || \
    fail CALYX_GATEBROKER_INSTALL_FILE_CAPABILITY_READ_FAILED \
      "cannot read file capabilities for $label: $path"
  [[ -z "$capabilities" ]] || \
    fail CALYX_GATEBROKER_INSTALL_FILE_CAPABILITIES \
      "$label must not carry file capabilities: path=$path actual=$capabilities"
}

group_contains_user() {
  local user="$1" group="$2" entry
  while IFS= read -r entry; do
    [[ "$entry" == "$group" ]] && return 0
  done < <(id -nG "$user" | tr ' ' '\n')
  return 1
}

verify_worker_account() {
  local entry name password uid gid gecos home shell primary_group shadow_entry
  local shadow_password linger_path client_group_entry client_gid
  local -a group_ids
  entry="$(getent passwd calyx-gate-worker)" || \
    fail CALYX_GATEBROKER_INSTALL_WORKER_MISSING "calyx-gate-worker does not exist"
  IFS=: read -r name password uid gid gecos home shell <<< "$entry"
  [[ "$name" == calyx-gate-worker && "$uid" =~ ^[0-9]+$ && "$uid" -ne 0 ]] || \
    fail CALYX_GATEBROKER_INSTALL_WORKER_ACCOUNT \
      "worker account name/uid is unsafe: name=$name uid=$uid"
  [[ "$home" == /nonexistent && "$shell" == /usr/sbin/nologin ]] || \
    fail CALYX_GATEBROKER_INSTALL_WORKER_ACCOUNT \
      "worker must use home=/nonexistent shell=/usr/sbin/nologin: home=$home shell=$shell"
  [[ ! -e "$home" && ! -L "$home" ]] || \
    fail CALYX_GATEBROKER_INSTALL_WORKER_HOME_EXISTS \
      "the worker home must remain absent: $home"
  primary_group="$(getent group "$gid")" || \
    fail CALYX_GATEBROKER_INSTALL_WORKER_GROUP_MISSING "worker primary gid does not exist: $gid"
  [[ "${primary_group%%:*}" == calyx-gate-worker ]] || \
    fail CALYX_GATEBROKER_INSTALL_WORKER_GROUP \
      "worker primary group must be calyx-gate-worker: gid=$gid entry=$primary_group"
  read -r -a group_ids <<< "$(id -G calyx-gate-worker)"
  [[ "${#group_ids[@]}" -eq 1 && "${group_ids[0]}" == "$gid" ]] || \
    fail CALYX_GATEBROKER_INSTALL_WORKER_SUPPLEMENTARY_GROUP \
      "worker must have no supplementary groups: gids=${group_ids[*]}"
  client_group_entry="$(getent group calyx-gate-clients)" || \
    fail CALYX_GATEBROKER_INSTALL_CLIENT_GROUP_MISSING "calyx-gate-clients does not exist"
  client_gid="${client_group_entry#*:}"
  client_gid="${client_gid#*:}"
  client_gid="${client_gid%%:*}"
  [[ "$client_gid" != "$gid" ]] || \
    fail CALYX_GATEBROKER_INSTALL_GROUP_ALIAS \
      "worker and client groups must not share gid=$gid"
  shadow_entry="$(getent shadow calyx-gate-worker)" || \
    fail CALYX_GATEBROKER_INSTALL_WORKER_SHADOW_MISSING \
      "worker has no local shadow entry"
  shadow_password="${shadow_entry#*:}"
  shadow_password="${shadow_password%%:*}"
  [[ "$shadow_password" == '!'* || "$shadow_password" == \** ]] || \
    fail CALYX_GATEBROKER_INSTALL_WORKER_UNLOCKED \
      "worker password field is not locked"
  for subordinate_file in /etc/subuid /etc/subgid; do
    if [[ -f "$subordinate_file" ]] && grep -Eq '^calyx-gate-worker:' "$subordinate_file"; then
      fail CALYX_GATEBROKER_INSTALL_WORKER_SUBORDINATE_IDS \
        "worker must not have subordinate IDs: $subordinate_file"
    fi
  done
  linger_path=/var/lib/systemd/linger/calyx-gate-worker
  [[ ! -e "$linger_path" && ! -L "$linger_path" ]] || \
    fail CALYX_GATEBROKER_INSTALL_WORKER_LINGER \
      "worker must not have linger enabled: $linger_path"
  ! systemctl is-active --quiet "user@$uid.service" || \
    fail CALYX_GATEBROKER_INSTALL_WORKER_MANAGER_ACTIVE \
      "worker user manager must not be active: user@$uid.service"
}

verify_identity_topology() {
  getent group calyx-gate-clients >/dev/null || \
    fail CALYX_GATEBROKER_INSTALL_CLIENT_GROUP_MISSING "calyx-gate-clients does not exist"
  getent passwd calyx-gate-worker >/dev/null || \
    fail CALYX_GATEBROKER_INSTALL_WORKER_MISSING "calyx-gate-worker does not exist"
  verify_worker_account
  id "$client_user" >/dev/null 2>&1 || \
    fail CALYX_GATEBROKER_INSTALL_CLIENT_MISSING "client user does not exist: $client_user"
  group_contains_user "$client_user" calyx-gate-clients || \
    fail CALYX_GATEBROKER_INSTALL_CLIENT_NOT_ENROLLED \
      "$client_user is not a member of calyx-gate-clients"
  ! group_contains_user calyx-gate-worker calyx-gate-clients || \
    fail CALYX_GATEBROKER_INSTALL_WORKER_IS_CLIENT \
      "calyx-gate-worker must not be a broker client"
}

verify_root_topology() {
  assert_root_owned_path_chain "$state_anchor" "broker state anchor"
  assert_dir_policy "$state_anchor" 711 "broker state anchor"
  assert_dir_policy "$state_anchor/objects" 711 "broker object container"
  assert_dir_policy "$shared_tmp" 711 "tmp shared root"
  assert_dir_policy "$shared_target" 711 "target shared root"
  assert_dir_policy "$state_private" 700 "broker private state"
  assert_dir_policy "$state_private/journal" 700 "broker journal directory"
  assert_dir_policy "$state_private/quarantine" 700 "broker quarantine container"
  assert_dir_policy "$private_tmp" 700 "tmp quarantine"
  assert_dir_policy "$private_target" 700 "target quarantine"
  assert_dir_policy /run/calyx-gatebrokerd 755 "broker runtime root"
  assert_execution_root_policy "$execution_source" croyse 755 "read-only source execution root"
}

verify_same_mount_pair() {
  local alias="$1" shared="$2" private="$3" shared_dev private_dev shared_mount private_mount
  local token private_probe shared_probe
  shared_dev="$(stat -Lc %d -- "$shared")"
  private_dev="$(stat -Lc %d -- "$private")"
  shared_mount="$(stat -Lc %m -- "$shared")"
  private_mount="$(stat -Lc %m -- "$private")"
  [[ "$shared_dev" == "$private_dev" && "$shared_mount" == "$private_mount" ]] || \
    fail CALYX_GATEBROKER_INSTALL_CROSS_MOUNT \
      "alias=$alias shared_dev=$shared_dev private_dev=$private_dev shared_mount=$shared_mount private_mount=$private_mount"

  token="$$-${RANDOM}-${RANDOM}"
  private_probe="$private/.install-rename-probe-$token"
  shared_probe="$shared/.install-rename-probe-$token"
  [[ ! -e "$private_probe" && ! -L "$private_probe" && ! -e "$shared_probe" && ! -L "$shared_probe" ]] || \
    fail CALYX_GATEBROKER_INSTALL_PROBE_COLLISION "alias=$alias token=$token"
  mkdir -m 0700 -- "$private_probe"
  if ! python3 - "$private_probe" "$shared_probe" <<'PY'
import os
import sys

private, shared = sys.argv[1:]
os.rename(private, shared)
os.rename(shared, private)
PY
  then
    rmdir -- "$shared_probe" 2>/dev/null || true
    rmdir -- "$private_probe" 2>/dev/null || true
    fail CALYX_GATEBROKER_INSTALL_RENAME_PROBE_FAILED \
      "rename must remain atomic and same-mount for alias=$alias"
  fi
  rmdir -- "$private_probe" || \
    fail CALYX_GATEBROKER_INSTALL_PROBE_CLEANUP_FAILED "probe remained for alias=$alias"
  printf 'gatebroker_root_verified alias=%s shared=%s private=%s device=%s mount=%s\n' \
    "$alias" "$shared" "$private" "$shared_dev" "$shared_mount"
}

verify_runtime() {
  assert_binary "$daemon_bin" calyx-gatebrokerd
  assert_binary "$shim_bin" calyx-gate-stage-shim
  assert_binary "$client_bin" calyx-gatectl
  assert_regular_policy "$config_dst" 600 "broker config"
  assert_regular_policy "$service_dst" 644 "broker service unit"
  assert_regular_policy "$socket_dst" 644 "broker socket unit"
  assert_regular_policy "$sysusers_dst" 644 "broker sysusers config"
  assert_regular_policy "$tmpfiles_dst" 644 "broker tmpfiles config"
  verify_identity_topology
  verify_root_topology
  verify_same_mount_pair tmp "$shared_tmp" "$private_tmp"
  verify_same_mount_pair target "$shared_target" "$private_target"
  "$daemon_bin" verify-config --config "$config_dst"
  systemd-analyze verify "$socket_dst" "$service_dst"
  systemctl is-active --quiet calyx-gatebrokerd.socket || \
    fail CALYX_GATEBROKER_INSTALL_SOCKET_INACTIVE "calyx-gatebrokerd.socket is not active"
  systemctl is-active --quiet calyx-gatebrokerd.service || \
    fail CALYX_GATEBROKER_INSTALL_SERVICE_INACTIVE "calyx-gatebrokerd.service is not active"
  for _ in {1..50}; do
    [[ -S "$socket_path" ]] && break
    sleep 0.1
  done
  [[ -S "$socket_path" && ! -L "$socket_path" ]] || \
    fail CALYX_GATEBROKER_INSTALL_SOCKET_MISSING "control socket is absent: $socket_path"
  local socket_policy journal_policy
  socket_policy="$(stat -Lc '%U:%G:%a' -- "$socket_path")"
  [[ "$socket_policy" == "root:calyx-gate-clients:660" ]] || \
    fail CALYX_GATEBROKER_INSTALL_SOCKET_POLICY \
      "expected root:calyx-gate-clients:660 at $socket_path; actual=$socket_policy"
  [[ -f "$journal_path" && ! -L "$journal_path" ]] || \
    fail CALYX_GATEBROKER_INSTALL_JOURNAL_MISSING "journal is absent or invalid: $journal_path"
  journal_policy="$(stat -Lc '%u:%g:%a' -- "$journal_path")"
  [[ "$journal_policy" == "0:0:600" ]] || \
    fail CALYX_GATEBROKER_INSTALL_JOURNAL_POLICY \
      "expected root:root:600 at $journal_path; actual=$journal_policy"
  "$client_bin" health --socket "$socket_path"
  echo "calyx-gatebrokerd deployment verified"
}

for source_path in "$service_src" "$socket_src" "$sysusers_src" "$tmpfiles_src" "$config_src"; do
  assert_source "$source_path"
done

if [[ "$mode" == verify ]]; then
  verify_runtime
  exit 0
fi

assert_binary "$daemon_bin" calyx-gatebrokerd
assert_binary "$shim_bin" calyx-gate-stage-shim
assert_binary "$client_bin" calyx-gatectl
id "$client_user" >/dev/null 2>&1 || \
  fail CALYX_GATEBROKER_INSTALL_CLIENT_MISSING "client user does not exist: $client_user"

# Source authority is read-only and is never created, re-owned, or chmodded by
# this installer. Mutable deletion roots are created only beneath /var/lib.
assert_execution_root_policy "$execution_source" croyse 755 "read-only source execution root"
assert_optional_dir_policy "$state_anchor" 711 "broker state anchor"
assert_optional_dir_policy "$state_anchor/objects" 711 "broker object container"
assert_optional_dir_policy "$shared_tmp" 711 "tmp shared root"
assert_optional_dir_policy "$shared_target" 711 "target shared root"
assert_optional_dir_policy "$state_private" 700 "broker private state"
assert_optional_dir_policy "$state_private/journal" 700 "broker journal directory"
assert_optional_dir_policy "$state_private/quarantine" 700 "broker quarantine container"
assert_optional_dir_policy "$private_tmp" 700 "tmp quarantine"
assert_optional_dir_policy "$private_target" 700 "target quarantine"
assert_optional_dir_policy /run/calyx-gatebrokerd 755 "broker runtime root"
assert_optional_dir_policy "$config_dir" 700 "broker config root"

for existing_file in "$service_dst:644:broker service unit" \
  "$socket_dst:644:broker socket unit" \
  "$sysusers_dst:644:broker sysusers config" \
  "$tmpfiles_dst:644:broker tmpfiles config"; do
  existing_path="${existing_file%%:*}"
  existing_tail="${existing_file#*:}"
  existing_mode="${existing_tail%%:*}"
  existing_label="${existing_tail#*:}"
  if [[ -e "$existing_path" || -L "$existing_path" ]]; then
    assert_regular_policy "$existing_path" "$existing_mode" "$existing_label"
  fi
done

for pair in "$service_src:$service_dst" "$socket_src:$socket_dst" \
  "$sysusers_src:$sysusers_dst" "$tmpfiles_src:$tmpfiles_dst"; do
  src="${pair%%:*}"
  dst="${pair#*:}"
  if systemctl is-active --quiet calyx-gatebrokerd.service \
    && { [[ ! -f "$dst" ]] || ! cmp -s -- "$src" "$dst"; }; then
    fail CALYX_GATEBROKER_INSTALL_ACTIVE_UNIT_CHANGE \
      "unit bytes changed while broker is active; finish all runs, stop the service, and rerun"
  fi
done

install -D -o root -g root -m 0644 "$sysusers_src" "$sysusers_dst"
systemd-sysusers "$sysusers_dst"
getent group calyx-gate-clients >/dev/null || \
  fail CALYX_GATEBROKER_INSTALL_CLIENT_GROUP_MISSING "sysusers did not create calyx-gate-clients"
getent passwd calyx-gate-worker >/dev/null || \
  fail CALYX_GATEBROKER_INSTALL_WORKER_MISSING "sysusers did not create calyx-gate-worker"
group_contains_user "$client_user" calyx-gate-clients || \
  usermod -a -G calyx-gate-clients "$client_user"

install -d -o root -g root -m 0700 "$config_dir"
if [[ ! -e "$config_dst" && ! -L "$config_dst" ]]; then
  install -o root -g root -m 0600 "$config_src" "$config_dst"
else
  assert_regular_policy "$config_dst" 600 "broker config"
  echo "preserving existing broker config: $config_dst"
fi

install -D -o root -g root -m 0644 "$tmpfiles_src" "$tmpfiles_dst"
systemd-tmpfiles --create "$tmpfiles_dst"
verify_root_topology
verify_same_mount_pair tmp "$shared_tmp" "$private_tmp"
verify_same_mount_pair target "$shared_target" "$private_target"
"$daemon_bin" verify-config --config "$config_dst"

install -o root -g root -m 0644 "$service_src" "$service_dst"
install -o root -g root -m 0644 "$socket_src" "$socket_dst"
systemd-analyze verify "$socket_dst" "$service_dst"
systemctl daemon-reload
systemctl enable calyx-gatebrokerd.socket calyx-gatebrokerd.service
if systemctl is-active --quiet calyx-gatebrokerd.service; then
  echo "calyx-gatebrokerd.service already active; preserving the running broker"
else
  systemctl restart calyx-gatebrokerd.socket
  systemctl start calyx-gatebrokerd.service
fi
verify_runtime
