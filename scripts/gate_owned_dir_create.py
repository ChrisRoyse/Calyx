#!/usr/bin/env python3
"""Create and identity-bind one unpublished Calyx gate directory."""

from __future__ import annotations

import argparse
import ctypes
import errno
import os
import secrets
import stat
import sys
from dataclasses import dataclass


RENAME_NOREPLACE = 1
MAX_TEXT = 4096
MAX_NAME = 240
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


def create(root: str, expected_root: Identity, name: str, uid: int) -> Identity:
    validate_text(root, "root")
    validate_text(name, "entry name", MAX_NAME)
    if not os.path.isabs(root) or name in (".", "..") or "/" in name:
        raise CreateError("entry must be one direct basename")
    root_fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    staging = f".calyx-create-{secrets.token_hex(16)}"
    entry_fd = -1
    staging_identity: Identity | None = None
    try:
        root_stat = os.fstat(root_fd)
        if Identity.from_stat(root_stat) != expected_root:
            raise CreateError("root identity changed before create")
        if root_stat.st_uid != uid or stat.S_IMODE(root_stat.st_mode) & 0o022:
            raise CreateError("root ownership or mode policy failed")
        os.mkdir(staging, 0o700, dir_fd=root_fd)
        entry_fd = os.open(
            staging,
            os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
            dir_fd=root_fd,
        )
        entry_stat = os.fstat(entry_fd)
        staging_identity = Identity.from_stat(entry_stat)
        if named_identity(root_fd, staging) != staging_identity:
            raise CreateError("staging identity changed before publication")
        if entry_stat.st_uid != uid or stat.S_IMODE(entry_stat.st_mode) != 0o700:
            raise CreateError("created entry ownership or mode policy failed")
        try:
            rename_noreplace(root_fd, staging, name)
        except OSError as error:
            if error.errno == errno.EEXIST:
                remove_exact_empty(root_fd, staging, staging_identity)
                raise FileExistsError(name) from error
            raise
        if named_identity(root_fd, name) != staging_identity:
            raise CreateError("published entry identity changed")
        if Identity.from_stat(os.fstat(root_fd)) != expected_root:
            raise CreateError("root identity changed during create")
        return staging_identity
    except Exception:
        if staging_identity is not None:
            remove_exact_empty(root_fd, staging, staging_identity)
        raise
    finally:
        if entry_fd >= 0:
            os.close(entry_fd)
        os.close(root_fd)


def main() -> int:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--root", required=True)
    parser.add_argument("--root-id", required=True)
    parser.add_argument("--name", required=True)
    parser.add_argument("--uid", required=True)
    try:
        args = parser.parse_args()
        root_identity = Identity.parse(args.root_id)
        entry_identity = create(args.root, root_identity, args.name, parse_uid(args.uid))
        print(f"CALYX_CREATED_V1 root={root_identity} entry={entry_identity}")
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
