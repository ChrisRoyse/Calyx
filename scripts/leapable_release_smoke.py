#!/usr/bin/env python3
"""Package and smoke-test the calyx-leapable release binary.

Source of truth for this script is the built binary and the vault bytes it writes
under a fresh data root. It fails closed if the binary does not boot, JSON-RPC
does not round-trip, or expected vault bytes are absent after the smoke session.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import queue
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import zipfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


MASTER_KEY_HEX = "00" * 32


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def repo_root() -> Path:
    result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    )
    return Path(result.stdout.strip()).resolve()


def git_value(args: list[str]) -> str:
    result = subprocess.run(
        ["git", *args],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    )
    return result.stdout.strip()


def workspace_version(root: Path) -> str:
    cargo_toml = root / "Cargo.toml"
    for line in cargo_toml.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("version ="):
            return stripped.split("=", 1)[1].strip().strip('"')
    raise RuntimeError("workspace version not found in Cargo.toml")


def read_line_with_timeout(pipe: Any, timeout_seconds: float) -> str:
    lines: queue.Queue[str] = queue.Queue(maxsize=1)

    def read() -> None:
        lines.put(pipe.readline())

    thread = threading.Thread(target=read, daemon=True)
    thread.start()
    try:
        return lines.get(timeout=timeout_seconds)
    except queue.Empty as exc:
        raise RuntimeError(f"timed out waiting {timeout_seconds}s for first response") from exc


def request(request_id: int, method: str, params: dict[str, Any]) -> str:
    return json.dumps(
        {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        },
        separators=(",", ":"),
    )


def list_files(root: Path) -> list[dict[str, Any]]:
    if not root.exists():
        return []
    entries: list[dict[str, Any]] = []
    for path in sorted(p for p in root.rglob("*") if p.is_file()):
        rel = path.relative_to(root).as_posix()
        entries.append(
            {
                "path": rel,
                "size_bytes": path.stat().st_size,
                "sha256": sha256_file(path),
            }
        )
    return entries


def run_build_info(binary: Path) -> dict[str, Any]:
    result = subprocess.run(
        [str(binary), "--build-info"],
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=20,
    )
    if result.returncode != 0:
        raise RuntimeError(
            "calyx-leapable --build-info failed "
            f"exit={result.returncode} stderr={result.stderr!r}"
        )
    return json.loads(result.stdout)


def run_smoke(binary: Path, platform: str) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix=f"calyx-leapable-release-{platform}-") as tmp:
        root = Path(tmp).resolve()
        vault_dir = root / "release-smoke.calyx"
        before = {
            "data_root": str(root),
            "vault_exists": vault_dir.exists(),
            "files": list_files(root),
        }
        env = os.environ.copy()
        env["CALYX_LEAPABLE_MASTER_KEY_HEX"] = MASTER_KEY_HEX
        proc = subprocess.Popen(
            [str(binary), "--data-dir", str(root)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
        )
        assert proc.stdin is not None
        assert proc.stdout is not None
        assert proc.stderr is not None

        first = request(1, "engine.info", {})
        rest = [
            request(
                2,
                "vault.create",
                {"vault_ref": "release-smoke", "ts": 1_785_800_000_000},
            ),
            request(
                3,
                "vault.stat",
                {"vault_ref": "release-smoke", "ts": 1_785_800_001_000},
            ),
        ]
        started = time.perf_counter()
        proc.stdin.write(first + "\n")
        proc.stdin.flush()
        first_line = read_line_with_timeout(proc.stdout, 15)
        cold_start_ms = round((time.perf_counter() - started) * 1000, 3)
        proc.stdin.write("\n".join(rest) + "\n")
        proc.stdin.close()
        remaining_stdout = proc.stdout.read()
        stderr = proc.stderr.read()
        try:
            exit_code = proc.wait(timeout=20)
        except subprocess.TimeoutExpired:
            proc.kill()
            raise RuntimeError("calyx-leapable did not exit after stdin EOF")

        stdout_lines = [first_line, *remaining_stdout.splitlines()]
        responses = [json.loads(line) for line in stdout_lines if line.strip()]
        errors = [response for response in responses if response.get("error") is not None]
        after = {
            "data_root": str(root),
            "vault_exists": vault_dir.exists(),
            "files": list_files(root),
        }
        if exit_code != 0:
            raise RuntimeError(f"calyx-leapable exited {exit_code}; stderr={stderr!r}")
        if len(responses) != 3:
            raise RuntimeError(f"expected 3 JSON-RPC responses, got {len(responses)}")
        if errors:
            raise RuntimeError(f"JSON-RPC smoke returned errors: {errors!r}")
        if "{" in stderr:
            raise RuntimeError(f"stderr contained protocol-looking JSON: {stderr!r}")
        if not after["vault_exists"]:
            raise RuntimeError(f"smoke vault bytes absent after run: {vault_dir}")
        if not after["files"]:
            raise RuntimeError("smoke data root has no file bytes after vault.create")

        return {
            "platform": platform,
            "binary": str(binary),
            "cold_start_ms": cold_start_ms,
            "exit_code": exit_code,
            "stdout": responses,
            "stderr": stderr,
            "before": before,
            "after": after,
        }


def ensure_executable(path: Path) -> None:
    if os.name != "nt":
        mode = path.stat().st_mode
        path.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def copy_release_tree(
    binary: Path,
    stage_dir: Path,
    binary_name: str,
    build_info: dict[str, Any],
    smoke: dict[str, Any],
    manifest: dict[str, Any],
) -> None:
    if stage_dir.exists():
        shutil.rmtree(stage_dir)
    stage_dir.mkdir(parents=True)
    shutil.copy2(binary, stage_dir / binary_name)
    ensure_executable(stage_dir / binary_name)
    (stage_dir / "build-info.json").write_text(
        json.dumps(build_info, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    (stage_dir / "smoke-readback.json").write_text(
        json.dumps(smoke, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    (stage_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def create_archive(stage_dir: Path, asset: Path, windows: bool) -> None:
    if asset.exists():
        asset.unlink()
    if windows:
        with zipfile.ZipFile(asset, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for path in sorted(stage_dir.rglob("*")):
                archive.write(path, path.relative_to(stage_dir.parent).as_posix())
    else:
        with tarfile.open(asset, "w:gz") as archive:
            for path in sorted(stage_dir.rglob("*")):
                archive.add(path, arcname=path.relative_to(stage_dir.parent).as_posix())


def write_provenance(
    path: Path,
    *,
    repo: str,
    commit: str,
    ref_name: str,
    run_id: str,
    run_attempt: str,
    platform: str,
    target: str,
    asset_name: str,
    asset_sha: str,
    workflow_ref: str,
) -> None:
    now = datetime.now(timezone.utc).isoformat()
    statement = {
        "_type": "https://in-toto.io/Statement/v1",
        "subject": [{"name": asset_name, "digest": {"sha256": asset_sha}}],
        "predicateType": "https://slsa.dev/provenance/v1",
        "predicate": {
            "buildDefinition": {
                "buildType": workflow_ref,
                "externalParameters": {
                    "platform": platform,
                    "target": target,
                    "ref": ref_name,
                },
                "internalParameters": {},
                "resolvedDependencies": [
                    {
                        "uri": f"git+https://github.com/{repo}",
                        "digest": {"gitCommit": commit},
                    }
                ],
            },
            "runDetails": {
                "builder": {"id": f"https://github.com/{repo}/actions/runs/{run_id}"},
                "metadata": {
                    "invocationId": run_id,
                    "runAttempt": run_attempt,
                    "startedOn": now,
                },
            },
        },
    }
    path.write_text(json.dumps(statement, sort_keys=True) + "\n", encoding="utf-8")


def write_github_output(values: dict[str, Any]) -> None:
    output = os.environ.get("GITHUB_OUTPUT")
    if not output:
        return
    with Path(output).open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            handle.write(f"{key}={value}\n")


def output_value(value: Any) -> str:
    if isinstance(value, Path):
        return value.as_posix()
    return str(value)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--platform", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--dist-dir", default=Path("dist"), type=Path)
    parser.add_argument("--asset-prefix", default="calyx-leapable")
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", "ChrisRoyse/Calyx-Dev"))
    parser.add_argument("--commit", default=os.environ.get("GITHUB_SHA"))
    parser.add_argument("--ref-name", default=os.environ.get("GITHUB_REF_NAME", "local"))
    parser.add_argument("--run-id", default=os.environ.get("GITHUB_RUN_ID", "local"))
    parser.add_argument("--run-attempt", default=os.environ.get("GITHUB_RUN_ATTEMPT", "0"))
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = repo_root()
    binary = args.binary.resolve()
    if not binary.exists():
        raise FileNotFoundError(f"release binary not found: {binary}")
    if not binary.is_file():
        raise FileNotFoundError(f"release binary is not a file: {binary}")
    ensure_executable(binary)

    commit = args.commit or git_value(["rev-parse", "HEAD"])
    version = workspace_version(root)
    dist_dir = args.dist_dir.resolve()
    dist_dir.mkdir(parents=True, exist_ok=True)

    windows = args.platform.startswith("windows")
    binary_name = "calyx-leapable.exe" if windows else "calyx-leapable"
    asset_stem = f"{args.asset_prefix}-{args.platform}"
    asset = dist_dir / (f"{asset_stem}.zip" if windows else f"{asset_stem}.tar.gz")
    stage_dir = dist_dir / asset_stem

    build_info = run_build_info(binary)
    smoke = run_smoke(binary, args.platform)
    binary_sha = sha256_file(binary)
    binary_size = binary.stat().st_size
    manifest = {
        "artifact": asset.name,
        "artifact_prefix": args.asset_prefix,
        "binary": binary_name,
        "binary_sha256": binary_sha,
        "binary_size_bytes": binary_size,
        "commit": commit,
        "platform": args.platform,
        "target": args.target,
        "version": version,
        "workflow": ".github/workflows/calyx-leapable-release.yml",
        "source_repo": args.repo,
        "build_info": build_info,
        "smoke_summary": {
            "cold_start_ms": smoke["cold_start_ms"],
            "file_count_after": len(smoke["after"]["files"]),
            "vault_exists_after": smoke["after"]["vault_exists"],
        },
    }
    copy_release_tree(binary, stage_dir, binary_name, build_info, smoke, manifest)
    create_archive(stage_dir, asset, windows)

    asset_sha = sha256_file(asset)
    checksum = Path(str(asset) + ".sha256")
    checksum.write_text(f"{asset_sha}  {asset.name}\n", encoding="utf-8")
    manifest_path = dist_dir / f"{asset_stem}.manifest.json"
    manifest["archive_sha256"] = asset_sha
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    provenance = Path(str(asset) + ".intoto.jsonl")
    write_provenance(
        provenance,
        repo=args.repo,
        commit=commit,
        ref_name=args.ref_name,
        run_id=args.run_id,
        run_attempt=args.run_attempt,
        platform=args.platform,
        target=args.target,
        asset_name=asset.name,
        asset_sha=asset_sha,
        workflow_ref=f"https://github.com/{args.repo}/.github/workflows/calyx-leapable-release.yml",
    )

    outputs = {
        "asset": asset,
        "checksum": checksum,
        "provenance": provenance,
        "manifest": manifest_path,
        "asset_sha256": asset_sha,
        "binary_size_bytes": binary_size,
        "cold_start_ms": smoke["cold_start_ms"],
    }
    output_strings = {key: output_value(value) for key, value in outputs.items()}
    write_github_output(output_strings)
    print(json.dumps(output_strings, indent=2))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"CALYX_LEAPABLE_RELEASE_SMOKE_FAILED: {exc}", file=sys.stderr)
        raise
