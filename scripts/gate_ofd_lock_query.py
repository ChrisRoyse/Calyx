#!/usr/bin/env python3
"""Query an inherited Linux OFD write lock without changing lock state."""

from __future__ import annotations

import ctypes
import fcntl
import os
import sys


class Flock(ctypes.Structure):
    _fields_ = [
        ("lock_type", ctypes.c_short),
        ("whence", ctypes.c_short),
        ("start", ctypes.c_long),
        ("length", ctypes.c_long),
        ("pid", ctypes.c_int),
    ]


libc = ctypes.CDLL(None, use_errno=True)


def main() -> int:
    if len(sys.argv) != 2 or not sys.argv[1].isascii() or not sys.argv[1].isdigit():
        return 64
    fd = int(sys.argv[1], 10)
    try:
        os.fstat(fd)
        lock = Flock(fcntl.F_WRLCK, os.SEEK_SET, 0, 0, 0)
        if libc.fcntl(fd, fcntl.F_OFD_GETLK, ctypes.byref(lock)) != 0:
            return 125
        lock_type = int(lock.lock_type)
    except (OSError, ValueError):
        return 125
    if lock_type == fcntl.F_UNLCK:
        print("unlocked")
        return 0
    if lock_type == fcntl.F_WRLCK:
        print("conflict")
        return 0
    return 125


if __name__ == "__main__":
    raise SystemExit(main())
