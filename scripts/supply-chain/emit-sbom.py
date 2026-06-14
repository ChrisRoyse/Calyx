#!/usr/bin/env python3
"""Emit a deterministic CycloneDX 1.6 SBOM from a Cargo.lock file.

PRD 30 §2 (supply chain: SBOM). Issue #596.

Self-contained: stdlib only (tomllib + json + hashlib + uuid). aiwonder has no
`cargo-cyclonedx`/`cargo-sbom` installed, and a release artifact must not depend
on a tool that may be absent — so we parse Cargo.lock natively and fail loud
rather than emit a partial or silently-degraded SBOM.

Determinism (the SBOM is itself a reproducible artifact):
  * components are sorted by (name, version);
  * the BOM `metadata.timestamp` is derived from SOURCE_DATE_EPOCH, never the
    wall clock;
  * `serialNumber` is a UUIDv5 of the component set, so identical input ->
    identical SBOM bytes.

CycloneDX 1.6 maps cleanly onto Cargo.lock:
  package name/version  -> component.name / component.version
  package source/version-> purl  pkg:cargo/<name>@<version>
  package checksum (sha256 hex of the .crate) -> component.hashes[SHA-256]

Error taxonomy (message on stderr, nonzero exit — fail closed, A16):
  CALYX_SBOM_PARSE_ERROR (2)  Cargo.lock missing / unreadable / invalid TOML /
                              a [[package]] entry missing name or version
  CALYX_SBOM_EMPTY       (3)  Cargo.lock parsed but contained zero packages

Usage:
  emit-sbom.py <Cargo.lock> [--output FILE] [--app-name N] [--app-version V]
  (writes to stdout when --output is omitted; prints "SBOM: <n> components" to
   stderr regardless)
"""

from __future__ import annotations

import argparse
import datetime as _dt
import hashlib
import json
import os
import sys
import uuid

try:
    import tomllib  # Python 3.11+
except ModuleNotFoundError as exc:  # pragma: no cover - environment guard
    sys.stderr.write(
        "CALYX_SBOM_PARSE_ERROR: tomllib unavailable (need Python >= 3.11): "
        f"{exc}\n"
    )
    raise SystemExit(2)

# Deterministic namespace for serialNumber derivation (a fixed random UUID).
_CALYX_SBOM_NS = uuid.UUID("6b3f9c2e-4a1d-5e8b-9f02-7c1a3d5e9b40")


def _fail(code: int, msg: str) -> "NoReturn":  # type: ignore[name-defined]
    sys.stderr.write(msg.rstrip() + "\n")
    raise SystemExit(code)


def _iso_from_epoch(epoch: int) -> str:
    """UTC ISO-8601 (seconds) from a Unix epoch — deterministic, no wall clock."""
    return (
        _dt.datetime.fromtimestamp(epoch, tz=_dt.timezone.utc)
        .strftime("%Y-%m-%dT%H:%M:%SZ")
    )


def _load_packages(lock_path: str) -> list[dict]:
    try:
        with open(lock_path, "rb") as fh:
            data = tomllib.load(fh)
    except FileNotFoundError:
        _fail(2, f"CALYX_SBOM_PARSE_ERROR: Cargo.lock not found at {lock_path}")
    except (tomllib.TOMLDecodeError, OSError) as exc:
        _fail(2, f"CALYX_SBOM_PARSE_ERROR: cannot parse {lock_path}: {exc}")

    packages = data.get("package", [])
    if not isinstance(packages, list):
        _fail(2, "CALYX_SBOM_PARSE_ERROR: [[package]] is not an array")
    return packages


def build_sbom(lock_path: str, app_name: str, app_version: str) -> tuple[dict, int]:
    packages = _load_packages(lock_path)

    components: list[dict] = []
    for idx, pkg in enumerate(packages):
        name = pkg.get("name")
        version = pkg.get("version")
        # Fail closed: a package without name/version means a malformed lock,
        # never a skipped-and-pretend-complete SBOM.
        if not name or not version:
            _fail(
                2,
                f"CALYX_SBOM_PARSE_ERROR: package #{idx} missing name/version: {pkg!r}",
            )
        purl = f"pkg:cargo/{name}@{version}"
        comp: dict = {
            "type": "library",
            "bom-ref": purl,
            "name": name,
            "version": version,
            "purl": purl,
        }
        checksum = pkg.get("checksum")
        if checksum:
            comp["hashes"] = [{"alg": "SHA-256", "content": checksum}]
        source = pkg.get("source")
        if source:
            comp["properties"] = [{"name": "cargo:source", "value": source}]
        components.append(comp)

    if not components:
        _fail(3, f"CALYX_SBOM_EMPTY: {lock_path} contained zero [[package]] entries")

    # Deterministic ordering.
    components.sort(key=lambda c: (c["name"], c["version"]))

    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "0"))
    timestamp = _iso_from_epoch(epoch)

    # serialNumber is a UUIDv5 over the (name@version) set => stable per input.
    digest_seed = "\n".join(c["purl"] for c in components)
    serial = uuid.uuid5(_CALYX_SBOM_NS, digest_seed)

    sbom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.6",
        "serialNumber": f"urn:uuid:{serial}",
        "version": 1,
        "metadata": {
            "timestamp": timestamp,
            "tools": {
                "components": [
                    {
                        "type": "application",
                        "name": "calyx-emit-sbom",
                        "version": "1.0.0",
                    }
                ]
            },
            "component": {
                "type": "application",
                "bom-ref": f"pkg:cargo/{app_name}@{app_version}",
                "name": app_name,
                "version": app_version,
            },
        },
        "components": components,
    }
    return sbom, len(components)


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description="Emit CycloneDX 1.6 SBOM from Cargo.lock")
    ap.add_argument("cargo_lock", help="path to Cargo.lock")
    ap.add_argument("--output", "-o", help="write SBOM JSON here (default: stdout)")
    ap.add_argument("--app-name", default="calyx", help="root component name")
    ap.add_argument("--app-version", default="0.1.0", help="root component version")
    args = ap.parse_args(argv)

    sbom, n = build_sbom(args.cargo_lock, args.app_name, args.app_version)
    # sort_keys=True for byte-stable output; trailing newline for POSIX text.
    text = json.dumps(sbom, indent=2, sort_keys=True) + "\n"

    if args.output:
        os.makedirs(os.path.dirname(os.path.abspath(args.output)), exist_ok=True)
        with open(args.output, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(text)
        sys.stderr.write(f"SBOM: {n} components -> {args.output}\n")
    else:
        sys.stdout.write(text)
        sys.stderr.write(f"SBOM: {n} components\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
