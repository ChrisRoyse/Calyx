#!/usr/bin/env bash
# Source-only public scratch mount-boundary regression for issue #1421.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from gate-wrapper-public-regression.sh" >&2
  exit 64
}

public_scratch_rel='crates/calyx-poly/C:\code\poly\target\fsv\issue029_ws_market_client_20260707'
public_scratch_path="$public_repo/$public_scratch_rel"
public_symlink_victim="$work/public/scratch-symlink-victim"
mkdir -p "$public_symlink_victim" "$(dirname "$public_scratch_path")"
printf 'public-scratch-symlink-referent\n' >"$public_symlink_victim/sentinel"
public_symlink_sha="$(sha256sum "$public_symlink_victim/sentinel" | awk '{print $1}')"
ln -s "$public_symlink_victim" "$public_scratch_path"
public_symlink_log="$work/public/scratch-symlink.log"
public_symlink_stages="$work/public/scratch-symlink-stages.log"
run_public_gate "$public_dev" "$public_repo" "$public_symlink_log" \
  "$public_symlink_stages" 0
[[ "$PUBLIC_RC" -eq 0 && ! -e "$public_scratch_path" && ! -L "$public_scratch_path" \
  && "$(sha256sum "$public_symlink_victim/sentinel" | awk '{print $1}')" == "$public_symlink_sha" \
  && "$(wc -l <"$public_symlink_stages" | tr -d ' ')" -eq 12 \
  && -z "$(git -C "$public_repo" status --porcelain)" ]] \
  && pass "public top-level scratch symlink is removed without changing its referent" \
  || {
    bad "public scratch symlink rc=$PUBLIC_RC was followed, retained, or skipped the workload"
    printf '    scratch-symlink: link=%s referent_sha=%s stages=%s status=%q\n' \
      "$(test -L "$public_scratch_path" && echo present || echo absent)" \
      "$(sha256sum "$public_symlink_victim/sentinel" | awk '{print $1}')" \
      "$(wc -l <"$public_symlink_stages" 2>/dev/null || echo missing)" \
      "$(git -C "$public_repo" status --porcelain)"
    sed -n '1,45p' "$public_symlink_log" | sed 's/^/    scratch-symlink-log: /'
  }
[[ ! -L "$public_scratch_path" ]] || rm -- "$public_scratch_path"
rm -rf -- "$public_symlink_victim"

mkdir -p "$public_scratch_path"
printf 'public-scratch-mount-sentinel\n' >"$public_scratch_path/sentinel"
public_scratch_sha="$(sha256sum "$public_scratch_path/sentinel" | awk '{print $1}')"
cat >"$public_dev/fakebin/findmnt" <<'STUB'
#!/usr/bin/env bash
value="${FAKE_MOUNT_TARGET:?}"
raw=""
for ((i = 0; i < ${#value}; i++)); do
  char="${value:i:1}"
  [[ "$char" != '\' ]] || char='\x5c'
  raw+="$char"
done
printf '%s\n' "$raw"
STUB
chmod +x "$public_dev/fakebin/findmnt"
set +e
FAKE_MOUNT_TARGET="$public_scratch_path/bind" \
  run_public_gate "$public_dev" "$public_repo" "$work/public/scratch-mount.log" \
    "$work/public/scratch-mount-stages.log" 0
public_scratch_mount_rc=$PUBLIC_RC
set -e
[[ "$public_scratch_mount_rc" -ne 0 \
  && "$(sha256sum "$public_scratch_path/sentinel" | awk '{print $1}')" == "$public_scratch_sha" ]] \
  && grep -Fq 'mountpoint at or below public scratch' "$work/public/scratch-mount.log" \
  && pass "public scratch cleanup refuses a nested mount and preserves bytes" \
  || bad "public scratch mount rc=$public_scratch_mount_rc was accepted or destructive"
rm -- "$public_dev/fakebin/findmnt"
rm -rf -- "$public_repo/crates"

swap_dev="$work/public-root-swap/calyx/dev"
swap_public="$work/public-root-swap/calyx/public-mirror"
swap_original="$work/public-root-swap/calyx/public-original"
setup_public_fixture "$swap_dev" "$swap_public"
real_git="$(command -v git)"
cat >"$swap_dev/fakebin/git" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
if [[ " $* " == *" remote get-url origin "* && ! -e "${FAKE_SWAP_DONE:?}" ]]; then
  origin="$("${REAL_GIT:?}" "$@")"
  mv -- "${FAKE_PUBLIC:?}" "${FAKE_ORIGINAL:?}"
  mkdir -p "$FAKE_PUBLIC"
  "$REAL_GIT" -C "$FAKE_PUBLIC" init -q -b main
  "$REAL_GIT" -C "$FAKE_PUBLIC" config user.name regression
  "$REAL_GIT" -C "$FAKE_PUBLIC" config user.email regression@example.invalid
  mkdir -p "$FAKE_PUBLIC/fuzz" "$FAKE_PUBLIC/${FAKE_SCRATCH_REL:?}"
  printf 'replacement fixture\n' >"$FAKE_PUBLIC/README.md"
  printf '[workspace]\nmembers=[]\n' >"$FAKE_PUBLIC/fuzz/Cargo.toml"
  printf 'replacement-public-root-bytes\n' >"$FAKE_PUBLIC/$FAKE_SCRATCH_REL/sentinel"
  "$REAL_GIT" -C "$FAKE_PUBLIC" add README.md fuzz/Cargo.toml
  "$REAL_GIT" -C "$FAKE_PUBLIC" commit -q -m replacement
  "$REAL_GIT" -C "$FAKE_PUBLIC" remote add origin https://github.com/ChrisRoyse/Calyx.git
  printf 'swapped\n' >"$FAKE_SWAP_DONE"
  printf '%s\n' "$origin"
  exit 0
fi
exec "${REAL_GIT:?}" "$@"
STUB
chmod +x "$swap_dev/fakebin/git"
swap_done="$work/public-root-swap/swap-done"
swap_log="$work/public-root-swap/gate.log"
swap_stages="$work/public-root-swap/stages.log"
export REAL_GIT="$real_git" FAKE_PUBLIC="$swap_public" FAKE_ORIGINAL="$swap_original" \
  FAKE_SWAP_DONE="$swap_done" FAKE_SCRATCH_REL="$public_scratch_rel"
set +e
run_public_gate "$swap_dev" "$swap_public" "$swap_log" "$swap_stages" 0
swap_rc=$PUBLIC_RC
set -e
unset REAL_GIT FAKE_PUBLIC FAKE_ORIGINAL FAKE_SWAP_DONE FAKE_SCRATCH_REL
swap_replacement_sha="$(sha256sum <"$swap_public/$public_scratch_rel/sentinel" | awk '{print $1}')"
[[ "$swap_rc" -eq 125 && -s "$swap_done" && ! -e "$swap_stages" \
  && -f "$swap_original/README.md" \
  && "$swap_replacement_sha" == "$(printf 'replacement-public-root-bytes\n' | sha256sum | awk '{print $1}')" ]] \
  && grep -Fq 'public checkout identity changed after operation' "$swap_log" \
  && pass "whole public-root substitution is rejected before scratch or workload mutation" \
  || {
    bad "public-root substitution rc=$swap_rc changed original/replacement state or reached workload"
    printf '    public-root-swap: done=%s stages=%s original=%s replacement_sha=%s\n' \
      "$(test -s "$swap_done" && echo yes || echo no)" \
      "$(test -e "$swap_stages" && wc -c <"$swap_stages" || echo absent)" \
      "$(test -f "$swap_original/README.md" && echo present || echo absent)" \
      "$swap_replacement_sha"
    sed -n '1,25p' "$swap_log" | sed 's/^/    public-root-swap-log: /'
  }

late_dev="$work/public-root-late-swap/calyx/dev"
late_public="$work/public-root-late-swap/calyx/public-mirror"
late_original="$work/public-root-late-swap/calyx/public-original"
late_replacement="$work/public-root-late-swap/calyx/public-replacement"
setup_public_fixture "$late_dev" "$late_public"
cp -a -- "$late_public" "$late_replacement"
printf 'late-root-replacement-sentinel\n' >"$late_replacement/replacement-sentinel"
"$real_git" -C "$late_replacement" add replacement-sentinel
"$real_git" -C "$late_replacement" commit -q -m late-replacement
late_sentinel_sha="$(sha256sum "$late_replacement/replacement-sentinel" | awk '{print $1}')"
real_stat="$(command -v stat)"
cat >"$late_dev/fakebin/stat" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
last="${!#}"
if [[ "$*" == *%d:%i* && "$last" == "${FAKE_PUBLIC:?}" ]]; then
  count=0
  [[ ! -f "${FAKE_STAT_COUNT:?}" ]] || read -r count <"$FAKE_STAT_COUNT"
  count=$((count + 1))
  printf '%s\n' "$count" >"$FAKE_STAT_COUNT"
  if [[ "$count" -eq 2 ]]; then
    value="$("${REAL_STAT:?}" "$@")"
    printf '%s\n' "$value"
    mv -- "$FAKE_PUBLIC" "${FAKE_ORIGINAL:?}"
    mv -- "${FAKE_REPLACEMENT:?}" "$FAKE_PUBLIC"
    printf 'swapped\n' >"${FAKE_SWAP_DONE:?}"
    exit 0
  fi
fi
exec "${REAL_STAT:?}" "$@"
STUB
chmod +x "$late_dev/fakebin/stat"
late_count="$work/public-root-late-swap/stat-count"
late_done="$work/public-root-late-swap/swap-done"
late_log="$work/public-root-late-swap/gate.log"
late_stages="$work/public-root-late-swap/stages.log"
export REAL_STAT="$real_stat" FAKE_PUBLIC="$late_public" FAKE_ORIGINAL="$late_original" \
  FAKE_REPLACEMENT="$late_replacement" FAKE_STAT_COUNT="$late_count" FAKE_SWAP_DONE="$late_done"
set +e
run_public_gate "$late_dev" "$late_public" "$late_log" "$late_stages" 0
late_rc=$PUBLIC_RC
set -e
unset REAL_STAT FAKE_PUBLIC FAKE_ORIGINAL FAKE_REPLACEMENT FAKE_STAT_COUNT FAKE_SWAP_DONE
late_after_sha="$(sha256sum "$late_public/replacement-sentinel" | awk '{print $1}')"
[[ "$late_rc" -eq 125 && -s "$late_done" && ! -e "$late_stages" \
  && -f "$late_original/README.md" && "$late_after_sha" == "$late_sentinel_sha" \
  && -z "$(git -C "$late_public" status --porcelain)" \
  && ! -e "$late_public/$public_scratch_rel" ]] \
  && grep -Fq 'public checkout identity changed before scratch cleanup' "$late_log" \
  && pass "post-origin public-root substitution is rejected before absent-leaf cleanup or workload" \
  || {
    bad "late public-root substitution rc=$late_rc touched replacement bytes or reached workload"
    printf '    late-public-root-swap: done=%s stages=%s original=%s replacement_sha=%s\n' \
      "$(test -s "$late_done" && echo yes || echo no)" \
      "$(test -e "$late_stages" && wc -c <"$late_stages" || echo absent)" \
      "$(test -f "$late_original/README.md" && echo present || echo absent)" "$late_after_sha"
    sed -n '1,30p' "$late_log" | sed 's/^/    late-public-root-swap-log: /'
  }
