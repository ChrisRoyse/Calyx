#!/usr/bin/env python3
"""Identity-bound, descriptor-relative removal for Calyx gate scratch trees."""

from __future__ import annotations

import argparse
import ctypes
import fcntl
import os
import secrets
import signal
import stat
import sys
import time
from dataclasses import dataclass


OPENAT2_NR = 437
AT_FDCWD = -100
RENAME_NOREPLACE = 1
RESOLVE_NO_XDEV = 0x01
RESOLVE_NO_MAGICLINKS = 0x02
RESOLVE_NO_SYMLINKS = 0x04
RESOLVE_BENEATH = 0x08
RESOLVE_SAFE = (
    RESOLVE_NO_XDEV | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_SYMLINKS | RESOLVE_BENEATH
)
OPEN_PATH = os.O_PATH | os.O_NOFOLLOW | os.O_CLOEXEC
OPEN_DIRECTORY = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
MAX_TEXT = 4096
MAX_TAG = 32
MAX_MESSAGE = 320
CARGO_TAG = "CACHEDIR.TAG"
CARGO_TAG_SIGNATURE = b"Signature: 8a477f597d28d172789f06886806bc55"
DEFERRED_SIGNALS = (signal.SIGHUP, signal.SIGINT, signal.SIGTERM)
MAX_REMOVED_ENTRIES = 2_000_000
MAX_REMOVAL_SECONDS = 300.0
MAX_REMOVAL_DEPTH = 128


class OpenHow(ctypes.Structure):
    _fields_ = [
        ("flags", ctypes.c_ulonglong),
        ("mode", ctypes.c_ulonglong),
        ("resolve", ctypes.c_ulonglong),
    ]


class Flock(ctypes.Structure):
    _fields_ = [
        ("lock_type", ctypes.c_short),
        ("whence", ctypes.c_short),
        ("start", ctypes.c_long),
        ("length", ctypes.c_long),
        ("pid", ctypes.c_int),
    ]


class SafeRemoveError(Exception):
    pass


class UsageError(SafeRemoveError):
    pass


class RemovalError(SafeRemoveError):
    def __init__(
        self, message: str, quarantine: str | None = None, recovery: str | None = None
    ) -> None:
        super().__init__(message)
        self.quarantine = quarantine
        self.recovery = recovery


@dataclass(frozen=True)
class Identity:
    device: int
    inode: int

    @classmethod
    def parse(cls, raw: str, label: str) -> "Identity":
        validate_scalar(raw, label)
        parts = raw.split(":")
        if len(parts) != 2 or not all(part.isascii() and part.isdigit() for part in parts):
            raise UsageError(f"invalid {label}")
        device, inode = (int(part, 10) for part in parts)
        if inode == 0:
            raise UsageError(f"invalid {label}")
        return cls(device, inode)

    @classmethod
    def from_stat(cls, value: os.stat_result) -> "Identity":
        return cls(value.st_dev, value.st_ino)

    def __str__(self) -> str:
        return f"{self.device}:{self.inode}"


class SignalDeferral:
    def __init__(self) -> None:
        self.pending: int | None = None
        self.previous: dict[signal.Signals, signal.Handlers] = {}

    def __enter__(self) -> "SignalDeferral":
        for signum in DEFERRED_SIGNALS:
            self.previous[signum] = signal.getsignal(signum)
            signal.signal(signum, self._record)
        return self

    def _record(self, signum: int, _frame: object) -> None:
        if self.pending is None:
            self.pending = signum

    def checkpoint(self) -> None:
        if self.pending is not None:
            raise RemovalError(f"cleanup stopped after deferred signal={self.pending}")

    def __exit__(self, _kind: object, _value: object, _traceback: object) -> None:
        for signum, previous in self.previous.items():
            signal.signal(signum, previous)

    def redeliver(self, tag: str) -> None:
        if self.pending is None:
            return
        emit_error(tag, f"deferred signal={self.pending} after cleanup")
        os.kill(os.getpid(), self.pending)
        os._exit(128 + self.pending)


@dataclass
class RemovalBudget:
    started: float
    removed: int = 0

    @classmethod
    def start(cls) -> "RemovalBudget":
        return cls(time.monotonic())

    def checkpoint(self, depth: int, deferred: SignalDeferral | None) -> None:
        if depth > MAX_REMOVAL_DEPTH:
            raise RemovalError("removal depth limit exceeded")
        if self.removed >= MAX_REMOVED_ENTRIES:
            raise RemovalError("removal entry limit exceeded")
        if time.monotonic() - self.started > MAX_REMOVAL_SECONDS:
            raise RemovalError("removal time limit exceeded")
        if deferred is not None:
            deferred.checkpoint()


libc = ctypes.CDLL(None, use_errno=True)
libc.syscall.restype = ctypes.c_long
fcntl_call = libc.fcntl
fcntl_call.restype = ctypes.c_int
renameat2_call = getattr(libc, "renameat2", None)
if renameat2_call is not None:
    renameat2_call.argtypes = [
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_uint,
    ]
    renameat2_call.restype = ctypes.c_int


def validate_scalar(raw: str, label: str, maximum: int = MAX_TEXT) -> None:
    if not raw or len(raw) > maximum or any(char in raw for char in ("\x00", "\r", "\n")):
        raise UsageError(f"invalid {label}")


def validate_tag(raw: str) -> str:
    validate_scalar(raw, "tag", MAX_TAG)
    if not all(char.isascii() and (char.isalnum() or char in "_-") for char in raw):
        raise UsageError("invalid tag")
    return raw


def relative_parts(raw: str) -> list[str]:
    validate_scalar(raw, "relative path")
    if raw.startswith("/"):
        raise UsageError("relative path must not be absolute")
    parts = raw.split("/")
    if any(part in ("", ".", "..") for part in parts):
        raise UsageError("relative path contains an empty or dot component")
    return parts


def parse_uid(raw: str) -> int:
    validate_scalar(raw, "uid")
    if not raw.isascii() or not raw.isdigit():
        raise UsageError("invalid uid")
    value = int(raw, 10)
    if value > 0xFFFFFFFE:
        raise UsageError("invalid uid")
    return value


def parse_mode_mask(raw: str | None) -> int:
    if raw is None:
        return 0
    validate_scalar(raw, "forbidden mode mask")
    if not raw.isascii() or any(char not in "01234567" for char in raw):
        raise UsageError("invalid forbidden mode mask")
    value = int(raw, 8)
    if value > 0o7777:
        raise UsageError("invalid forbidden mode mask")
    return value


def bytes_name(name: str) -> bytes:
    try:
        return os.fsencode(name)
    except UnicodeEncodeError as error:
        raise SafeRemoveError("path encoding failed") from error


def openat2(directory_fd: int, name: str, flags: int) -> int:
    how = OpenHow(flags=flags, mode=0, resolve=RESOLVE_SAFE)
    result = libc.syscall(
        OPENAT2_NR,
        directory_fd,
        ctypes.c_char_p(bytes_name(name)),
        ctypes.byref(how),
        ctypes.sizeof(how),
    )
    if result < 0:
        error_number = ctypes.get_errno()
        raise OSError(error_number, os.strerror(error_number), name)
    return int(result)


def ofd_lock_state(fd: int) -> int:
    lock = Flock(fcntl.F_WRLCK, os.SEEK_SET, 0, 0, 0)
    if fcntl_call(fd, fcntl.F_OFD_GETLK, ctypes.byref(lock)) != 0:
        error_number = ctypes.get_errno()
        raise OSError(error_number, os.strerror(error_number))
    return int(lock.lock_type)


def verify_gate_lock(args: argparse.Namespace, uid: int) -> None:
    fd_raw = getattr(args, "gate_lock_fd", None)
    identity_raw = getattr(args, "gate_lock_id", None)
    if fd_raw is None and identity_raw is None:
        return
    if fd_raw is None or identity_raw is None or not fd_raw.isascii() or not fd_raw.isdigit():
        raise UsageError("incomplete gate lock proof")
    fd = int(fd_raw, 10)
    expected = Identity.parse(identity_raw, "gate lock identity")
    lock_stat = os.fstat(fd)
    verify_identity(lock_stat, expected, "gate lock")
    verify_policy(lock_stat, uid, 0o022, "gate lock")
    if not stat.S_ISREG(lock_stat.st_mode):
        raise SafeRemoveError("gate lock is not a regular file")
    probe_fd = os.open(f"/proc/self/fd/{fd}", os.O_RDWR | os.O_CLOEXEC)
    try:
        candidate = ofd_lock_state(fd)
        probe_before = ofd_lock_state(probe_fd)
        probe_after = ofd_lock_state(probe_fd)
    finally:
        os.close(probe_fd)
    if candidate != fcntl.F_UNLCK or probe_before != fcntl.F_WRLCK or probe_after != fcntl.F_WRLCK:
        raise SafeRemoveError("gate lock OFD ownership proof failed")


def rename_noreplace(directory_fd: int, source: str, destination: str) -> None:
    if renameat2_call is None:
        raise SafeRemoveError("renameat2 is unavailable")
    result = renameat2_call(
        directory_fd,
        ctypes.c_char_p(bytes_name(source)),
        directory_fd,
        ctypes.c_char_p(bytes_name(destination)),
        RENAME_NOREPLACE,
    )
    if result != 0:
        error_number = ctypes.get_errno()
        raise OSError(error_number, os.strerror(error_number), source)


def verify_policy(
    value: os.stat_result,
    expected_uid: int,
    forbidden_mode: int,
    label: str,
) -> None:
    if value.st_uid != expected_uid:
        raise SafeRemoveError(f"{label} uid mismatch")
    if stat.S_IMODE(value.st_mode) & forbidden_mode:
        raise SafeRemoveError(f"{label} has forbidden mode bits")


def verify_identity(value: os.stat_result, expected: Identity, label: str) -> None:
    if Identity.from_stat(value) != expected:
        raise SafeRemoveError(f"{label} identity mismatch")


def random_quarantine(tag: str) -> str:
    return f".calyx-remove-{tag}-{secrets.token_hex(12)}"


def checked_quarantine(
    parent_fd: int,
    source: str,
    opened_stat: os.stat_result,
    tag: str,
) -> str:
    for _attempt in range(16):
        quarantine = random_quarantine(tag)
        try:
            rename_noreplace(parent_fd, source, quarantine)
        except FileExistsError:
            continue
        except OSError as error:
            raise RemovalError("atomic quarantine failed") from error
        try:
            quarantined_fd = openat2(parent_fd, quarantine, OPEN_PATH)
        except OSError as error:
            raise RemovalError("cannot open quarantined entry", quarantine) from error
        try:
            expected = Identity.from_stat(opened_stat)
            actual = Identity.from_stat(os.fstat(quarantined_fd))
            if actual != expected:
                try:
                    restore_quarantine(parent_fd, quarantine, source, actual)
                except Exception as recovery_error:
                    raise RemovalError(
                        "quarantined entry identity mismatch; "
                        f"recovery failed: {recovery_error}",
                        quarantine,
                    ) from recovery_error
                raise RemovalError(
                    "quarantined entry identity mismatch", recovery=source
                )
        finally:
            os.close(quarantined_fd)
        return quarantine
    raise RemovalError("cannot allocate quarantine name")


def verify_named_identity(
    parent_fd: int, name: str, expected: Identity, directory: bool
) -> None:
    flags = OPEN_DIRECTORY if directory else OPEN_PATH
    try:
        opened_fd = openat2(parent_fd, name, flags)
    except OSError as error:
        raise RemovalError("cannot reopen quarantined entry", name) from error
    try:
        verify_identity(os.fstat(opened_fd), expected, "quarantined entry")
    finally:
        os.close(opened_fd)


def restore_quarantine(
    parent_fd: int, quarantine: str, source: str, expected: Identity
) -> None:
    verify_named_identity(parent_fd, quarantine, expected, False)
    try:
        rename_noreplace(parent_fd, quarantine, source)
    except OSError as error:
        raise RemovalError("cannot restore quarantined entry", quarantine) from error
    verify_named_identity(parent_fd, source, expected, False)


def first_entry(directory_fd: int) -> str | None:
    with os.scandir(directory_fd) as entries:
        for entry in entries:
            if entry.name not in (".", ".."):
                return entry.name
    return None


def remove_opened_entry(
    parent_fd: int,
    source: str,
    opened_fd: int,
    opened_stat: os.stat_result,
    tag: str,
    budget: RemovalBudget,
    deferred: SignalDeferral | None,
    depth: int,
) -> None:
    quarantine: str | None = None
    removed_before = budget.removed
    expected = Identity.from_stat(opened_stat)
    try:
        quarantine = checked_quarantine(parent_fd, source, opened_stat, tag)
        if stat.S_ISDIR(opened_stat.st_mode):
            try:
                directory_fd = openat2(parent_fd, quarantine, OPEN_DIRECTORY)
            except OSError as error:
                raise RemovalError("cannot open quarantined directory", quarantine) from error
            try:
                verify_identity(os.fstat(directory_fd), expected, "directory")
                drain_directory(directory_fd, tag, budget, deferred, depth + 1)
            finally:
                os.close(directory_fd)
            budget.checkpoint(depth, deferred)
            verify_named_identity(parent_fd, quarantine, expected, True)
            try:
                os.rmdir(quarantine, dir_fd=parent_fd)
            except OSError as error:
                raise RemovalError("cannot remove empty quarantined directory", quarantine) from error
        else:
            budget.checkpoint(depth, deferred)
            verify_named_identity(parent_fd, quarantine, expected, False)
            try:
                os.unlink(quarantine, dir_fd=parent_fd)
            except OSError as error:
                raise RemovalError("cannot unlink quarantined entry", quarantine) from error
        budget.removed += 1
    except Exception as error:
        if quarantine is not None and budget.removed == removed_before:
            try:
                restore_quarantine(parent_fd, quarantine, source, expected)
            except Exception as recovery_error:
                raise RemovalError(
                    f"{error}; recovery failed: {recovery_error}", quarantine
                ) from error
            raise RemovalError(str(error), recovery=source) from error
        if isinstance(error, RemovalError):
            if error.quarantine is None:
                error.quarantine = quarantine
            raise
        raise RemovalError("cannot drain quarantined entry", quarantine) from error
    finally:
        os.close(opened_fd)


def drain_directory(
    directory_fd: int,
    tag: str,
    budget: RemovalBudget,
    deferred: SignalDeferral | None,
    depth: int,
) -> None:
    while True:
        budget.checkpoint(depth, deferred)
        name = first_entry(directory_fd)
        if name is None:
            return
        try:
            child_fd = openat2(directory_fd, name, OPEN_PATH)
        except FileNotFoundError:
            continue
        except OSError as error:
            raise RemovalError("cannot open child beneath quarantine") from error
        child_stat = os.fstat(child_fd)
        remove_opened_entry(
            directory_fd, name, child_fd, child_stat, tag, budget, deferred, depth
        )


def require_cargo_marker_or_empty(directory_fd: int, uid: int) -> None:
    try:
        marker_fd = openat2(directory_fd, CARGO_TAG, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    except FileNotFoundError:
        if first_entry(directory_fd) is None:
            return
        raise SafeRemoveError("nonempty entry has no canonical Cargo target marker")
    except OSError as error:
        raise SafeRemoveError("Cargo marker cannot be opened safely") from error
    try:
        marker_stat = os.fstat(marker_fd)
        if (
            not stat.S_ISREG(marker_stat.st_mode)
            or marker_stat.st_uid != uid
            or marker_stat.st_size > 4096
        ):
            raise SafeRemoveError(
                "Cargo marker policy failed: "
                f"type={stat.S_IFMT(marker_stat.st_mode):o} uid={marker_stat.st_uid} "
                f"mode={stat.S_IMODE(marker_stat.st_mode):o} size={marker_stat.st_size}"
            )
        marker_bytes = os.read(marker_fd, len(CARGO_TAG_SIGNATURE) + 1)
        if not marker_bytes.startswith(CARGO_TAG_SIGNATURE) or marker_bytes[
            len(CARGO_TAG_SIGNATURE) :
        ] not in (b"", b"\n"):
            raise SafeRemoveError("Cargo marker signature is invalid")
    finally:
        os.close(marker_fd)


def open_root(path: str) -> int:
    validate_scalar(path, "root")
    if not os.path.isabs(path):
        raise UsageError("root must be absolute")
    try:
        return os.open(path, OPEN_DIRECTORY)
    except OSError as error:
        raise SafeRemoveError("cannot open authorization root") from error


def remove(
    args: argparse.Namespace, deferred: SignalDeferral | None = None
) -> tuple[str, Identity | None]:
    parts = relative_parts(args.relative)
    root_identity = Identity.parse(args.root_id, "root identity")
    expected_identity = (
        None if args.entry_id is None else Identity.parse(args.entry_id, "entry identity")
    )
    uid = parse_uid(args.uid)
    verify_gate_lock(args, uid)
    budget = RemovalBudget.start()
    forbidden_mode = parse_mode_mask(args.forbid_mode_mask)
    root_uid = uid if args.root_uid is None else parse_uid(args.root_uid)
    root_forbidden_mode = (
        forbidden_mode
        if args.root_forbid_mode_mask is None
        else parse_mode_mask(args.root_forbid_mode_mask)
    )
    if (expected_identity is None) == (not args.expect_absent):
        raise UsageError("exactly one of --entry-id and --expect-absent is required")
    root_fd = open_root(args.root)
    try:
        root_stat = os.fstat(root_fd)
        verify_identity(root_stat, root_identity, "authorization root")
        verify_policy(root_stat, root_uid, root_forbidden_mode, "authorization root")
        parent_name = "/".join(parts[:-1])
        if parent_name:
            try:
                parent_fd = openat2(root_fd, parent_name, OPEN_DIRECTORY)
            except OSError as error:
                raise SafeRemoveError("cannot open entry parent beneath root") from error
        else:
            parent_fd = os.dup(root_fd)
        try:
            parent_stat = os.fstat(parent_fd)
            if parent_name:
                verify_policy(parent_stat, uid, forbidden_mode, "entry parent")
            else:
                verify_policy(parent_stat, root_uid, root_forbidden_mode, "entry parent")
            try:
                entry_fd = openat2(parent_fd, parts[-1], OPEN_PATH)
            except FileNotFoundError:
                if args.expect_absent:
                    return "absent", None
                raise SafeRemoveError("expected entry is absent")
            except OSError as error:
                raise SafeRemoveError("cannot open entry beneath root") from error
            entry_stat = os.fstat(entry_fd)
            if args.expect_absent:
                os.close(entry_fd)
                raise SafeRemoveError("entry exists but absence was required")
            if expected_identity is None:
                raise UsageError("entry identity is required for removal")
            try:
                verify_identity(entry_stat, expected_identity, "entry")
                verify_policy(entry_stat, uid, forbidden_mode, "entry")
                if args.kind == "directory" and not stat.S_ISDIR(entry_stat.st_mode):
                    raise SafeRemoveError("entry is not a directory")
                if args.require_cargo_marker_or_empty:
                    if not stat.S_ISDIR(entry_stat.st_mode):
                        raise SafeRemoveError("Cargo marker policy requires a directory")
                    directory_fd = openat2(parent_fd, parts[-1], OPEN_DIRECTORY)
                    try:
                        verify_identity(
                            os.fstat(directory_fd), expected_identity, "Cargo target entry"
                        )
                        require_cargo_marker_or_empty(directory_fd, uid)
                    finally:
                        os.close(directory_fd)
            except Exception:
                os.close(entry_fd)
                raise
            remove_opened_entry(
                parent_fd, parts[-1], entry_fd, entry_stat, args.tag, budget, deferred, 0
            )
            return "removed", expected_identity
        finally:
            os.close(parent_fd)
    finally:
        os.close(root_fd)


class BoundedParser(argparse.ArgumentParser):
    def error(self, _message: str) -> None:
        raise UsageError("invalid arguments")


def parser() -> argparse.ArgumentParser:
    result = BoundedParser(
        prog="gate_safe_rmtree.py",
        description="Remove one identity-bound entry beneath an authorization root.",
    )
    result.add_argument("--root", required=True)
    result.add_argument("--root-id", required=True)
    result.add_argument("--relative", required=True)
    identity = result.add_mutually_exclusive_group(required=True)
    identity.add_argument("--entry-id")
    identity.add_argument("--expect-absent", action="store_true")
    result.add_argument("--kind", choices=("directory", "any"), required=True)
    result.add_argument("--uid", required=True)
    result.add_argument("--forbid-mode-mask")
    result.add_argument("--root-uid")
    result.add_argument("--root-forbid-mode-mask")
    result.add_argument("--require-cargo-marker-or-empty", action="store_true")
    result.add_argument("--tag", required=True)
    result.add_argument("--gate-lock-fd")
    result.add_argument("--gate-lock-id")
    return result


def emit_error(tag: str, message: str) -> None:
    safe = " ".join(str(message).replace("\x00", "?").split())[:MAX_MESSAGE]
    print(f"gate-safe-rmtree: tag={tag} status=error detail={safe}", file=sys.stderr)


def main() -> int:
    try:
        args = parser().parse_args()
        args.tag = validate_tag(args.tag)
    except (SafeRemoveError, SystemExit) as error:
        if isinstance(error, SystemExit):
            return int(error.code)
        emit_error("invalid", str(error))
        return 2
    deferred = SignalDeferral()
    try:
        with deferred:
            status, identity = remove(args, deferred)
        suffix = "" if identity is None else f" identity={identity}"
        print(f"gate-safe-rmtree: tag={args.tag} status={status}{suffix}")
        sys.stdout.flush()
        deferred.redeliver(args.tag)
        return 0
    except RemovalError as error:
        detail = str(error)
        if error.quarantine is not None:
            detail += f" quarantine={error.quarantine}"
        if error.recovery is not None:
            detail += f" recovered={error.recovery}"
        emit_error(args.tag, detail)
        deferred.redeliver(args.tag)
        return 1
    except Exception as error:
        emit_error(args.tag, str(error))
        deferred.redeliver(args.tag)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
