#!/usr/bin/env python3
"""Create and identity-bind one unpublished Calyx gate directory."""

from __future__ import annotations

import argparse
import ctypes
import errno
import hashlib
import os
import signal
import stat
import sys
from dataclasses import dataclass


RENAME_NOREPLACE = 1
MAX_TEXT = 4096
MAX_NAME = 240
MAX_PUBLICATION_BYTES = 1024
BLOCKED_SIGNALS = {signal.SIGHUP, signal.SIGINT, signal.SIGTERM}
libc = ctypes.CDLL(None, use_errno=True)
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


class CreateError(Exception):
    pass


@dataclass(frozen=True)
class Identity:
    device: int
    inode: int

    @classmethod
    def parse(cls, raw: str) -> "Identity":
        parts = raw.split(":")
        if len(parts) != 2 or not all(part.isascii() and part.isdigit() for part in parts):
            raise CreateError("invalid root identity")
        value = cls(*(int(part, 10) for part in parts))
        if value.inode == 0:
            raise CreateError("invalid root identity")
        return value

    @classmethod
    def from_stat(cls, value: os.stat_result) -> "Identity":
        return cls(value.st_dev, value.st_ino)

    def __str__(self) -> str:
        return f"{self.device}:{self.inode}"


def validate_text(raw: str, label: str, maximum: int = MAX_TEXT) -> None:
    if not raw or len(raw) > maximum or any(char in raw for char in ("\x00", "\r", "\n")):
        raise CreateError(f"invalid {label}")


def parse_uid(raw: str) -> int:
    validate_text(raw, "uid")
    if not raw.isascii() or not raw.isdigit():
        raise CreateError("invalid uid")
    return int(raw, 10)


def bytes_name(name: str) -> bytes:
    try:
        return os.fsencode(name)
    except UnicodeEncodeError as error:
        raise CreateError("path encoding failed") from error


def rename_noreplace(root_fd: int, source: str, destination: str) -> None:
    if renameat2_call is None:
        raise CreateError("renameat2 is unavailable")
    result = renameat2_call(
        root_fd,
        ctypes.c_char_p(bytes_name(source)),
        root_fd,
        ctypes.c_char_p(bytes_name(destination)),
        RENAME_NOREPLACE,
    )
    if result != 0:
        number = ctypes.get_errno()
        raise OSError(number, os.strerror(number), destination)


def named_identity(root_fd: int, name: str) -> Identity:
    value = os.stat(name, dir_fd=root_fd, follow_symlinks=False)
    return Identity.from_stat(value)


def remove_exact_empty(root_fd: int, name: str, expected: Identity) -> bool:
    try:
        if named_identity(root_fd, name) != expected:
            return False
        os.rmdir(name, dir_fd=root_fd)
        return True
    except OSError:
        return False


def write_record(fd: int, record: str, uid: int, require_private: bool) -> None:
    payload = record.encode("ascii")
    if require_private:
        value = os.fstat(fd)
        is_fifo = stat.S_ISFIFO(value.st_mode)
        is_regular = stat.S_ISREG(value.st_mode)
        if not (is_fifo or is_regular) or value.st_uid != uid:
            raise CreateError("identity publication fd policy failed")
        if stat.S_IMODE(value.st_mode) & 0o077:
            raise CreateError("identity publication fd is not private")
        if is_fifo and len(payload) > os.fpathconf(fd, "PC_PIPE_BUF"):
            raise CreateError("identity publication record is not atomic")
        if is_regular:
            position = os.lseek(fd, 0, os.SEEK_CUR)
            if position != value.st_size or position + len(payload) > MAX_PUBLICATION_BYTES:
                raise CreateError("identity publication file exceeded its bounded append policy")
    written = os.write(fd, payload)
    if written != len(payload):
        raise CreateError("identity publication record was truncated")


def create(
    root: str,
    expected_root: Identity,
    name: str,
    uid: int,
    publish_fd: int | None,
) -> Identity:
    validate_text(root, "root")
    validate_text(name, "entry name", MAX_NAME)
    if not os.path.isabs(root) or name in (".", "..") or "/" in name:
        raise CreateError("entry must be one direct basename")
    root_fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    name_digest = hashlib.sha256(bytes_name(name)).hexdigest()
    staging = f".calyx-create-{name_digest}"
    entry_fd = -1
    staging_identity: Identity | None = None
    staging_created = False
    final_published = False
    previous_mask: set[signal.Signals] | None = None
    try:
        root_stat = os.fstat(root_fd)
        if Identity.from_stat(root_stat) != expected_root:
            raise CreateError("root identity changed before create")
        if root_stat.st_uid != uid or stat.S_IMODE(root_stat.st_mode) & 0o022:
            raise CreateError("root ownership or mode policy failed")
        previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, BLOCKED_SIGNALS)
        staging_created = True
        try:
            os.mkdir(staging, 0o700, dir_fd=root_fd)
        except OSError:
            staging_created = False
            raise
        # Staging creation is now committed for rollback.
        staging_stat = os.stat(staging, dir_fd=root_fd, follow_symlinks=False)
        staging_identity = Identity.from_stat(staging_stat)
        if not stat.S_ISDIR(staging_stat.st_mode):
            raise CreateError("created staging entry is not a directory")
        if staging_stat.st_uid != uid or stat.S_IMODE(staging_stat.st_mode) != 0o700:
            raise CreateError("created staging ownership or mode policy failed")
        if publish_fd is not None:
            write_record(
                publish_fd,
                f"CALYX_CREATE_PREPARED_V1 root={expected_root} "
                f"entry={staging_identity} staging={staging}\n",
                uid,
                True,
            )
        # From this point the parent can recover the exact staging identity.
        entry_fd = os.open(
            staging,
            os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
            dir_fd=root_fd,
        )
        entry_stat = os.fstat(entry_fd)
        if Identity.from_stat(entry_stat) != staging_identity:
            raise CreateError("opened staging identity changed")
        if named_identity(root_fd, staging) != staging_identity:
            raise CreateError("staging identity changed before publication")
        if entry_stat.st_uid != uid or stat.S_IMODE(entry_stat.st_mode) != 0o700:
            raise CreateError("created entry ownership or mode policy failed")
        try:
            rename_noreplace(root_fd, staging, name)
        except OSError as error:
            if error.errno == errno.EEXIST:
                raise FileExistsError(name) from error
            raise
        final_published = True
        if named_identity(root_fd, name) != staging_identity:
            raise CreateError("published entry identity changed")
        if Identity.from_stat(os.fstat(root_fd)) != expected_root:
            raise CreateError("root identity changed during create")
        write_record(
            sys.stdout.fileno(),
            f"CALYX_CREATED_V1 root={expected_root} entry={staging_identity}\n",
            uid,
            False,
        )
        if publish_fd is not None:
            write_record(
                publish_fd,
                f"CALYX_CREATE_ID_V1 root={expected_root} entry={staging_identity}\n",
                uid,
                True,
            )
        return staging_identity
    except BaseException as error:
        if staging_identity is not None:
            rollback_name = name if final_published else staging
            if not remove_exact_empty(root_fd, rollback_name, staging_identity):
                raise CreateError(
                    f"exact rollback failed for {rollback_name} identity={staging_identity}"
                ) from error
        elif staging_created:
            try:
                value = os.stat(staging, dir_fd=root_fd, follow_symlinks=False)
                if not stat.S_ISDIR(value.st_mode) or value.st_uid != uid:
                    raise CreateError("unbound staging rollback policy failed")
                if stat.S_IMODE(value.st_mode) != 0o700:
                    raise CreateError("unbound staging rollback mode failed")
                os.rmdir(staging, dir_fd=root_fd)
            except BaseException as rollback_error:
                raise CreateError(f"unbound staging rollback failed for {staging}") from rollback_error
        raise
    finally:
        if previous_mask is not None:
            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
        if entry_fd >= 0:
            os.close(entry_fd)
        os.close(root_fd)


def main() -> int:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--root", required=True)
    parser.add_argument("--root-id", required=True)
    parser.add_argument("--name", required=True)
    parser.add_argument("--uid", required=True)
    parser.add_argument("--publish-fd", type=int)
    try:
        args = parser.parse_args()
        root_identity = Identity.parse(args.root_id)
        uid = parse_uid(args.uid)
        if args.publish_fd is not None and args.publish_fd < 0:
            raise CreateError("invalid identity publication fd")
        create(args.root, root_identity, args.name, uid, args.publish_fd)
        return 0
    except FileExistsError:
        print("gate-owned-dir-create: collision", file=sys.stderr)
        return 75
    except Exception as error:
        detail = " ".join(str(error).replace("\x00", "?").split())[:320]
        print(f"gate-owned-dir-create: status=error detail={detail}", file=sys.stderr)
        return 125


if __name__ == "__main__":
    raise SystemExit(main())
