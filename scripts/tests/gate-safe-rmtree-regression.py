#!/usr/bin/env python3
"""Adversarial real-filesystem regressions for gate_safe_rmtree.py (#1421)."""

from __future__ import annotations

import argparse
import errno
import hashlib
import importlib.util
import os
import shutil
import signal
import sys
from pathlib import Path
from types import ModuleType, SimpleNamespace


class Failure(RuntimeError):
    pass


def load_helper(path: Path) -> ModuleType:
    spec = importlib.util.spec_from_file_location("gate_safe_rmtree_under_test", path)
    if spec is None or spec.loader is None:
        raise Failure("cannot load safe-removal helper")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def identity(path: Path) -> str:
    value = path.stat(follow_symlinks=False)
    return f"{value.st_dev}:{value.st_ino}"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def mkdir(path: Path) -> None:
    path.mkdir(parents=True)
    path.chmod(0o700)


def removal_args(root: Path, entry: Path, *, kind: str = "directory") -> SimpleNamespace:
    return SimpleNamespace(
        root=str(root),
        root_id=identity(root),
        relative=str(entry.relative_to(root)),
        entry_id=identity(entry),
        expect_absent=False,
        kind=kind,
        uid=str(os.getuid()),
        root_uid=str(os.getuid()),
        forbid_mode_mask="0022",
        root_forbid_mode_mask="0022",
        require_cargo_marker_or_empty=False,
        tag="safe-rmtree-regression",
    )


def require(condition: bool, message: str) -> None:
    if not condition:
        raise Failure(message)


def case_nested_symlink(module: ModuleType, work: Path) -> None:
    root = work / "symlink-root"
    valuable = work / "valuable"
    entry = root / "victim"
    mkdir(root)
    mkdir(valuable)
    mkdir(entry)
    sentinel = valuable / "sentinel"
    sentinel.write_text("valuable-outside-bytes\n", encoding="utf-8")
    before = digest(sentinel)
    (entry / "outside-link").symlink_to(valuable, target_is_directory=True)
    status, removed = module.remove(removal_args(root, entry))
    require(status == "removed" and removed is not None, "symlink case did not remove")
    require(not entry.exists(), "symlink-bearing victim remains")
    require(sentinel.is_file() and digest(sentinel) == before, "symlink referent changed")
    print("  PASS: nested symlink is unlinked without traversing or changing its referent")


def case_root_substitution(module: ModuleType, work: Path) -> None:
    root = work / "root-substitution"
    original = work / "root-substitution-original"
    entry = root / "victim"
    mkdir(root)
    mkdir(entry)
    (entry / "sentinel").write_text("original-root-bytes\n", encoding="utf-8")
    args = removal_args(root, entry)
    root.rename(original)
    mkdir(root)
    replacement = root / "victim"
    mkdir(replacement)
    (replacement / "sentinel").write_text("replacement-root-bytes\n", encoding="utf-8")
    try:
        module.remove(args)
    except module.SafeRemoveError:
        pass
    else:
        raise Failure("root substitution was accepted")
    require((original / "victim" / "sentinel").is_file(), "original root bytes lost")
    require((replacement / "sentinel").is_file(), "replacement root bytes lost")
    print("  PASS: authorization-root substitution preserves original and replacement bytes")


def case_post_open_swap(module: ModuleType, work: Path) -> None:
    root = work / "post-open-root"
    victim = root / "victim"
    replacement = root / "replacement"
    parked = root / "parked-original"
    mkdir(root)
    mkdir(victim)
    mkdir(replacement)
    original_file = victim / "sentinel"
    replacement_file = replacement / "sentinel"
    original_file.write_text("post-open-original\n", encoding="utf-8")
    replacement_file.write_text("post-open-replacement\n", encoding="utf-8")
    original_sha = digest(original_file)
    replacement_sha = digest(replacement_file)
    args = removal_args(root, victim)
    original_id = args.entry_id
    replacement_id = identity(replacement)
    real_rename = module.rename_noreplace
    swapped = False

    def swap_then_rename(parent_fd: int, source: str, quarantine: str) -> None:
        nonlocal swapped
        if not swapped:
            require(source == "victim", "unexpected post-open source")
            require(
                quarantine.startswith(".calyx-remove-"),
                "unexpected post-open quarantine",
            )
            os.rename(
                "victim", "parked-original", src_dir_fd=parent_fd, dst_dir_fd=parent_fd
            )
            os.rename(
                "replacement", "victim", src_dir_fd=parent_fd, dst_dir_fd=parent_fd
            )
            swapped = True
        real_rename(parent_fd, source, quarantine)

    module.rename_noreplace = swap_then_rename
    error = None
    try:
        module.remove(args)
    except module.RemovalError as caught:
        error = caught
    finally:
        module.rename_noreplace = real_rename
    require(error is not None, "post-open swap was accepted")
    require(str(error) == "quarantined entry identity mismatch", "wrong swap error")
    require(error.recovery == "victim", "post-open swap lacked source recovery")
    require(error.quarantine is None, "post-open swap left a hidden quarantine")
    require(identity(parked) == original_id, "opened original identity changed")
    require(digest(parked / "sentinel") == original_sha, "opened original was deleted")
    require(identity(victim) == replacement_id, "replacement identity was not restored")
    require(digest(victim / "sentinel") == replacement_sha, "replacement was deleted")
    require(not replacement.exists(), "consumed replacement pathname reappeared")
    require(not tuple(root.glob(".calyx-remove-*")), "hidden quarantine remains")
    print("  PASS: post-open pathname swap is detected and both identities remain byte-exact")


def case_failure_quarantine(module: ModuleType, work: Path) -> None:
    root = work / "failure-root"
    entry = root / "victim"
    mkdir(root)
    mkdir(entry)
    sentinel = entry / "sentinel"
    sentinel.write_text("recovery-visible-bytes\n", encoding="utf-8")
    before = digest(sentinel)
    args = removal_args(root, entry)
    real_drain = module.drain_directory

    def fail_drain(
        _directory_fd: int,
        _tag: str,
        _budget: object,
        _deferred: object,
        _depth: int,
    ) -> None:
        raise module.RemovalError("injected drain failure")

    module.drain_directory = fail_drain
    error = None
    try:
        module.remove(args)
    except module.RemovalError as caught:
        error = caught
    finally:
        module.drain_directory = real_drain
    require(error is not None and error.recovery == "victim", "drain failure lacked rollback proof")
    require(digest(entry / "sentinel") == before, "rolled-back recovery bytes changed")
    status, removed = module.remove(removal_args(root, entry))
    require(status == "removed" and removed is not None, "recovered entry could not be retried")
    require(not entry.exists(), "recovered entry remains after retry")
    print("  PASS: pre-drain failure rolls back byte-exactly and a retry removes the entry")


def case_partial_drain_recovery(module: ModuleType, work: Path) -> None:
    root = work / "partial-root"
    entry = root / "victim"
    outside = work / "partial-outside"
    mkdir(root)
    mkdir(entry)
    mkdir(outside)
    (entry / "a").write_text("first-child\n", encoding="utf-8")
    (entry / "b").write_text("second-child\n", encoding="utf-8")
    outside_sentinel = outside / "sentinel"
    outside_sentinel.write_text("partial-outside-bytes\n", encoding="utf-8")
    outside_sha = digest(outside_sentinel)
    real_remove = module.remove_opened_entry
    removed_once = False

    def fail_after_one(*args: object, **kwargs: object) -> None:
        nonlocal removed_once
        real_remove(*args, **kwargs)
        if not removed_once:
            removed_once = True
            raise module.RemovalError("injected failure after one deletion")

    module.remove_opened_entry = fail_after_one
    error = None
    try:
        module.remove(removal_args(root, entry))
    except module.RemovalError as caught:
        error = caught
    finally:
        module.remove_opened_entry = real_remove
    require(error is not None and error.quarantine is not None, "partial drain lacked quarantine")
    quarantine = root / error.quarantine
    require(not entry.exists() and quarantine.is_dir(), "partial tree was not quarantined")
    require(digest(outside_sentinel) == outside_sha, "partial failure changed outside bytes")
    status, removed = module.remove(removal_args(root, quarantine))
    require(status == "removed" and removed is not None, "quarantine recovery retry failed")
    require(not quarantine.exists(), "quarantine remains after recovery retry")
    print("  PASS: partial drain reports an exact quarantine that a fresh retry removes")


def case_late_insertion(module: ModuleType, work: Path) -> None:
    root = work / "late-root"
    entry = root / "victim"
    mkdir(root)
    mkdir(entry)
    real_drain = module.drain_directory

    def insert_after_drain(*args: object, **kwargs: object) -> None:
        real_drain(*args, **kwargs)
        os.mkdir("late-child", dir_fd=int(args[0]))

    module.drain_directory = insert_after_drain
    error = None
    try:
        module.remove(removal_args(root, entry))
    except module.RemovalError as caught:
        error = caught
    finally:
        module.drain_directory = real_drain
    require(error is not None and error.recovery == "victim", "late insertion was not rolled back")
    require((entry / "late-child").is_dir(), "late inserted bytes were lost")
    module.remove(removal_args(root, entry))
    print("  PASS: insertion before final rmdir fails closed and restores the public name")


def case_cargo_marker(module: ModuleType, work: Path) -> None:
    root = work / "marker-root"
    entry = root / "target-lookalike"
    mkdir(root)
    mkdir(entry)
    (entry / "payload").write_text("marker-payload\n", encoding="utf-8")
    marker = entry / "CACHEDIR.TAG"
    marker.write_text("not-a-cargo-marker\n", encoding="utf-8")
    args = removal_args(root, entry)
    args.require_cargo_marker_or_empty = True
    try:
        module.remove(args)
    except module.SafeRemoveError:
        pass
    else:
        raise Failure("invalid Cargo marker authorized deletion")
    require((entry / "payload").is_file(), "invalid marker case changed payload")
    marker.write_text(
        "Signature: 8a477f597d28d172789f06886806bc55\n", encoding="utf-8"
    )
    status, _removed = module.remove(args)
    require(status == "removed" and not entry.exists(), "canonical Cargo marker was rejected")
    print("  PASS: only the canonical Cargo cache signature authorizes a nonempty target")


def case_no_xdev(module: ModuleType, work: Path) -> None:
    root = work / "xdev-root"
    entry = root / "victim"
    child = entry / "mountlike"
    mkdir(root)
    mkdir(entry)
    mkdir(child)
    sentinel = child / "sentinel"
    sentinel.write_text("xdev-preserved\n", encoding="utf-8")
    before = digest(sentinel)
    real_openat2 = module.openat2

    def reject_cross_device(directory_fd: int, name: str, flags: int) -> int:
        if name == "mountlike":
            raise OSError(errno.EXDEV, "injected cross-device boundary")
        return real_openat2(directory_fd, name, flags)

    module.openat2 = reject_cross_device
    error = None
    try:
        module.remove(removal_args(root, entry))
    except module.RemovalError as caught:
        error = caught
    finally:
        module.openat2 = real_openat2
    require(error is not None and error.recovery == "victim", "EXDEV was not rolled back")
    require(digest(sentinel) == before, "EXDEV boundary changed child bytes")
    module.remove(removal_args(root, entry))
    print("  PASS: cross-device traversal refusal preserves bytes and restores the entry")


def case_pending_signal(module: ModuleType, work: Path) -> None:
    root = work / "signal-root"
    entry = root / "victim"
    mkdir(root)
    mkdir(entry)
    (entry / "sentinel").write_text("signal-recovery\n", encoding="utf-8")
    deferred = module.SignalDeferral()
    deferred.pending = signal.SIGTERM
    error = None
    try:
        module.remove(removal_args(root, entry), deferred)
    except module.RemovalError as caught:
        error = caught
    require(error is not None and error.recovery == "victim", "pending signal was not bounded")
    require((entry / "sentinel").is_file(), "pending signal lost recovery bytes")
    module.remove(removal_args(root, entry))
    print("  PASS: a pending signal aborts at the next checkpoint with byte-exact rollback")


def case_scandir_failure(module: ModuleType, work: Path) -> None:
    root = work / "scandir-root"
    entry = root / "victim"
    mkdir(root)
    mkdir(entry)
    sentinel = entry / "sentinel"
    sentinel.write_text("scandir-recovery\n", encoding="utf-8")
    before = digest(sentinel)
    real_scandir = module.os.scandir
    failed = False

    def fail_once(path: object) -> object:
        nonlocal failed
        if not failed and isinstance(path, int):
            failed = True
            raise OSError(errno.EIO, "injected scandir failure")
        return real_scandir(path)

    module.os.scandir = fail_once
    error = None
    try:
        module.remove(removal_args(root, entry))
    except module.RemovalError as caught:
        error = caught
    finally:
        module.os.scandir = real_scandir
    require(error is not None and error.recovery == "victim", "scandir failure was not rolled back")
    require(digest(sentinel) == before, "scandir failure changed recovery bytes")
    module.remove(removal_args(root, entry))
    print("  PASS: scandir producer failure restores byte-exact public recovery state")


def case_malformed_relative(module: ModuleType, work: Path) -> None:
    root = work / "malformed-root"
    entry = root / "victim"
    mkdir(root)
    mkdir(entry)
    args = removal_args(root, entry)
    args.relative = "../victim"
    try:
        module.remove(args)
    except module.UsageError:
        pass
    else:
        raise Failure("parent traversal relative path was accepted")
    require(entry.is_dir(), "malformed relative path changed entry")
    print("  PASS: parent traversal is rejected before entry mutation")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("helper", type=Path)
    parser.add_argument("work", type=Path)
    args = parser.parse_args()
    if args.work.exists():
        shutil.rmtree(args.work)
    args.work.mkdir(mode=0o700, parents=True)
    module = load_helper(args.helper.resolve())
    cases = (
        case_nested_symlink,
        case_root_substitution,
        case_post_open_swap,
        case_failure_quarantine,
        case_partial_drain_recovery,
        case_late_insertion,
        case_cargo_marker,
        case_no_xdev,
        case_pending_signal,
        case_scandir_failure,
        case_malformed_relative,
    )
    try:
        for case in cases:
            case(module, args.work)
    except (Failure, OSError) as error:
        print(f"  FAIL: {error}", file=sys.stderr)
        return 1
    finally:
        shutil.rmtree(args.work, ignore_errors=True)
    print("ALL 11 SAFE-REMOVAL ADVERSARIAL SCENARIOS PASSED")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
