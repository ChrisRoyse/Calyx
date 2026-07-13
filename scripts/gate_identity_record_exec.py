#!/usr/bin/env python3
"""Run one gate creator with a private, size-bounded publication descriptor."""

from __future__ import annotations

import os
import re
import resource
import select
import selectors
import stat
import subprocess
import sys
import time


CREATE_PREPARED = re.compile(
    rb"CALYX_CREATE_PREPARED_V1 root=([0-9]+:[0-9]+) "
    rb"entry=([0-9]+:[0-9]+) staging=(\.calyx-create-[0-9a-f]{64})\n"
)
CREATE_FINAL = re.compile(
    rb"CALYX_CREATE_ID_V1 root=([0-9]+:[0-9]+) "
    rb"entry=([0-9]+:[0-9]+)\n"
)
LAUNCH_RECORD = re.compile(
    rb"CALYX_GATE_LAUNCH_ID_V1 pid=([1-9][0-9]*) starttime=([1-9][0-9]*)\n"
)


def write_all(fd: int, payload: bytes) -> None:
    offset = 0
    while offset < len(payload):
        written = os.write(fd, payload[offset:])
        if written <= 0:
            raise OSError("short diagnostic write")
        offset += written


def set_child_limit(max_bytes: int) -> None:
    resource.setrlimit(resource.RLIMIT_FSIZE, (max_bytes, max_bytes))


def terminate_child_bounded(process: subprocess.Popen[bytes], timeout: float = 1.0) -> int | None:
    status = process.poll()
    if status is not None:
        return status
    try:
        process.kill()
    except ProcessLookupError:
        pass
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        status = process.poll()
        if status is not None:
            return status
        time.sleep(0.01)
    return process.poll()


def parse_fd(value: str) -> int:
    fd = int(value)
    if fd < 3:
        raise ValueError("descriptor must be at least 3")
    return fd


def parse_max_bytes(value: str) -> int:
    maximum = int(value)
    if not 128 <= maximum <= 4096:
        raise ValueError("byte limit must be between 128 and 4096")
    return maximum


def parse_timeout(value: str) -> float:
    timeout = float(value)
    if not 0.1 <= timeout <= 30:
        raise ValueError("timeout must be between 0.1 and 30 seconds")
    return timeout


def validate_fd(fd: int, expected_type: str) -> None:
    value = os.fstat(fd)
    type_ok = stat.S_ISREG(value.st_mode) if expected_type == "regular" else stat.S_ISFIFO(value.st_mode)
    if not type_ok or value.st_uid != os.getuid():
        raise OSError(f"invalid {expected_type} publication descriptor")
    if stat.S_IMODE(value.st_mode) & 0o077:
        raise OSError("publication descriptor is not private")


def read_bounded(fd: int, max_bytes: int, timeout: float) -> bytes:
    deadline = time.monotonic() + timeout
    payload = bytearray()
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("publication record timed out")
        readable, _, _ = select.select([fd], [], [], remaining)
        if not readable:
            raise TimeoutError("publication record timed out")
        chunk = os.read(fd, max_bytes + 1 - len(payload))
        if not chunk:
            return bytes(payload)
        payload.extend(chunk)
        if len(payload) > max_bytes:
            raise ValueError("publication record exceeds byte limit")


def run_creator(args: list[str]) -> int:
    if len(args) < 4:
        return 64
    try:
        record_fd = parse_fd(args[0])
        max_bytes = parse_max_bytes(args[1])
        timeout = parse_timeout(args[2])
    except ValueError:
        return 64
    validate_fd(record_fd, "regular")
    environment = os.environ.copy()
    environment["CALYX_CREATE_ID_FD"] = str(record_fd)
    process = subprocess.Popen(
        args[3:],
        close_fds=True,
        env=environment,
        pass_fds=(record_fd,),
        preexec_fn=lambda: set_child_limit(max_bytes),
        stderr=subprocess.PIPE,
        stdout=subprocess.PIPE,
    )
    selector = selectors.DefaultSelector()
    assert process.stdout is not None and process.stderr is not None
    selector.register(process.stdout, selectors.EVENT_READ, sys.stdout.fileno())
    selector.register(process.stderr, selectors.EVENT_READ, sys.stderr.fileno())
    deadline = time.monotonic() + timeout
    exit_deadline: float | None = None
    timed_out = False
    try:
        while selector.get_map():
            now = time.monotonic()
            if process.poll() is None and now >= deadline:
                timed_out = True
                break
            if process.poll() is not None:
                if exit_deadline is None:
                    exit_deadline = now + 1.0
                elif now >= exit_deadline:
                    break
            for key, _ in selector.select(0.1):
                chunk = os.read(key.fileobj.fileno(), 65536)
                if chunk:
                    write_all(key.data, chunk)
                else:
                    selector.unregister(key.fileobj)
    except (OSError, subprocess.SubprocessError):
        terminate_child_bounded(process)
        raise
    finally:
        selector.close()
        try:
            process.stdout.close()
        except OSError:
            pass
        try:
            process.stderr.close()
        except OSError:
            pass
    status = terminate_child_bounded(process) if timed_out else process.poll()
    if status is None:
        status = terminate_child_bounded(process)
    if timed_out:
        print("gate-identity-record-exec: creator timed out", file=sys.stderr)
        return 125
    if status is None:
        print("gate-identity-record-exec: creator could not be boundedly reaped", file=sys.stderr)
        return 125
    return status if status >= 0 else 128 - status


def read_create(args: list[str]) -> int:
    if len(args) != 3:
        return 64
    fd = parse_fd(args[0])
    maximum = parse_max_bytes(args[1])
    timeout = parse_timeout(args[2])
    validate_fd(fd, "regular")
    payload = read_bounded(fd, maximum, timeout)
    prepared = CREATE_PREPARED.match(payload)
    final = CREATE_FINAL.fullmatch(payload)
    if final is not None:
        root, entry = (part.decode("ascii") for part in final.groups())
        print(f"CALYX_CREATE_CAPTURE_V1 root={root} entry={entry}")
        return 0
    if prepared is not None:
        root, entry, staging = (part.decode("ascii") for part in prepared.groups())
        remainder = payload[prepared.end() :]
        final = CREATE_FINAL.fullmatch(remainder)
        if final is not None and final.groups() == prepared.groups()[:2]:
            print(
                f"CALYX_CREATE_CAPTURE_V1 root={root} entry={entry} staging={staging}"
            )
            return 0
        print(
            f"CALYX_CREATE_PREPARED_V1 root={root} entry={entry} staging={staging}"
        )
    print("gate-identity-record-exec: creator identity record is malformed", file=sys.stderr)
    return 125


def read_launch(args: list[str]) -> int:
    if len(args) != 4:
        return 64
    fd = parse_fd(args[0])
    maximum = parse_max_bytes(args[1])
    timeout = parse_timeout(args[2])
    expected_pid = args[3]
    if not expected_pid.isascii() or not expected_pid.isdigit() or expected_pid.startswith("0"):
        return 64
    validate_fd(fd, "fifo")
    deadline = time.monotonic() + timeout
    payload = bytearray()
    published = False
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            print("gate-identity-record-exec: launch identity record timed out", file=sys.stderr)
            return 125
        readable, _, _ = select.select([fd], [], [], remaining)
        if not readable:
            print("gate-identity-record-exec: launch identity record timed out", file=sys.stderr)
            return 125
        chunk = os.read(fd, maximum + 1 - len(payload))
        if not chunk:
            break
        payload.extend(chunk)
        if not published and b"\n" in payload:
            first = bytes(payload[: payload.index(b"\n") + 1])
            match = LAUNCH_RECORD.fullmatch(first)
            if match is not None and match.group(1).decode("ascii") == expected_pid:
                print(match.group(2).decode("ascii"), flush=True)
                published = True
        if len(payload) > maximum:
            print("gate-identity-record-exec: launch identity record exceeds byte limit", file=sys.stderr)
            return 125
    match = LAUNCH_RECORD.fullmatch(bytes(payload))
    if match is None or match.group(1).decode("ascii") != expected_pid:
        print("gate-identity-record-exec: launch identity record is malformed", file=sys.stderr)
        return 125
    if not published:
        print(match.group(2).decode("ascii"))
    return 0


def run(argv: list[str]) -> int:
    if len(argv) < 2:
        return 64
    if argv[1] == "exec":
        return run_creator(argv[2:])
    if argv[1] == "read-create":
        return read_create(argv[2:])
    if argv[1] == "read-launch":
        return read_launch(argv[2:])
    return 64


if __name__ == "__main__":
    try:
        raise SystemExit(run(sys.argv))
    except (OSError, TimeoutError, ValueError, subprocess.SubprocessError) as error:
        print(f"gate-identity-record-exec: {error}", file=sys.stderr)
        raise SystemExit(125) from error
