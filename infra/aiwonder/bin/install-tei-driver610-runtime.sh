#!/usr/bin/env bash
# Install aiwonder TEI systemd drop-ins that mount the driver-610 entrypoint.
set -euo pipefail

if [[ "$(id -u)" -ne 0 ]]; then
    echo "ERROR: run as root (use sudo)" >&2
    exit 1
fi

restart=0
if [[ "${1:-}" == "--restart" ]]; then
    restart=1
elif [[ $# -gt 0 ]]; then
    echo "usage: $0 [--restart]" >&2
    exit 2
fi

repo_root="${CALYX_REPO:-/home/croyse/calyx/repo}"
entrypoint="$repo_root/infra/aiwonder/bin/tei-driver610-entrypoint.sh"
if [[ ! -x "$entrypoint" ]]; then
    echo "ERROR: TEI entrypoint is not executable: $entrypoint" >&2
    exit 1
fi

write_dropin() {
    local service="$1" env_file="$2" volume="$3" port="$4"
    local dir="/etc/systemd/system/${service}.service.d"
    install -d -m 755 "$dir"
    cat > "$dir/calyx-driver610-runtime.conf" <<EOF
[Service]
ExecStart=
ExecStart=/usr/bin/docker run --rm --name ${service} --gpus all --pull=never --env-file ${env_file} -e HF_TOKEN -e LD_LIBRARY_PATH=/usr/local/cuda/lib64:/usr/local/cuda/lib64 -e NVIDIA_VISIBLE_DEVICES=all -e NVIDIA_DRIVER_CAPABILITIES=compute,utility -v ${entrypoint}:/calyx/tei-driver610-entrypoint.sh:ro -v ${volume}:/data -p 127.0.0.1:${port}:80 --entrypoint /calyx/tei-driver610-entrypoint.sh ghcr.io/leapable/tei-blackwell:phase0-wrapper
EOF
}

write_dropin "leapable-tei-general" "/etc/leapable/tei-general.env" "/zfs/hot/models/huggingface" "8088"
write_dropin "leapable-tei-reranker" "/etc/leapable/tei-reranker.env" "/zfs/hot/models/huggingface-reranker-gte" "8089"
write_dropin "leapable-tei-legal" "/etc/leapable/tei-legal.env" "/zfs/hot/models/huggingface" "8090"

systemctl daemon-reload
if [[ "$restart" -eq 1 ]]; then
    systemctl restart leapable-tei-general leapable-tei-reranker leapable-tei-legal
fi
