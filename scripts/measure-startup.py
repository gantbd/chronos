"""Measure one Chronos binary against private, synthetic Unix socket fixtures."""

import argparse
from contextlib import contextmanager
import hashlib
import json
from pathlib import Path
import selectors
import shutil
import socket
import statistics
import subprocess
import tempfile
import time


@contextmanager
def service(binary, state, timeout, *flags):
    start = time.monotonic_ns()
    child = subprocess.Popen(
        [binary, state, *flags], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE
    )
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(child.stderr, selectors.EVENT_READ)
            if not selector.select(timeout):
                raise RuntimeError(f"readiness deadline exceeded in {state.name}")
            announcement = child.stderr.readline(4096).strip()
        if announcement != b"Chronos local service ready":
            raise RuntimeError(f"service refused startup: {announcement!r}")
        yield start, time.monotonic_ns()
    finally:
        if child.poll() is None:
            child.terminate()
        try:
            code = child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()
            raise RuntimeError("fixture shutdown deadline exceeded")
        finally:
            child.stderr.close()
    if code != 0:
        raise RuntimeError(f"fixture shutdown exited with {code}")


def call(state, operation, *, version=1, **arguments):
    frame = {"version": version, "command": {"operation": operation, **arguments}}
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(3)
        connection.connect(str(state / "chronos.sock"))
        connection.sendall(json.dumps(frame).encode() + b"\n")
        with connection.makefile("rb") as response:
            line = response.readline(262145)
    if len(line) > 262144 or not line.endswith(b"\n"):
        raise RuntimeError("invalid response framing")
    reply = json.loads(line)
    if reply["version"] != version:
        raise RuntimeError("unexpected response version")
    return reply["outcome"]


def populate(state, count):
    revision = 0
    for first in range(0, count, 128):
        tasks = [
            {
                "id": f"startup-{index}",
                "priority": 0,
                "dependencies": [],
                "capabilities": [],
                "max_attempts": 1,
                "retry_safety": "fence_unknown",
            }
            for index in range(first, min(first + 128, count))
        ]
        result = call(
            state, "submit_graph",
            expected={"kind": "finite", "value": str(revision)}, tasks=tasks,
        )
        if result["outcome"] != "applied":
            raise RuntimeError(f"fixture submission refused: {result}")
        revision += 1
    return revision


def measure(binary, root, mode, trials, timeout, flags, revision, *, fresh_copy=False):
    rows = []
    binary_hash = digest(binary) if fresh_copy else None
    for trial in range(trials):
        state = root / str(trial) if mode == "new" else root
        executable = binary
        if fresh_copy:
            executable = root / f"candidate-{trial}"
            shutil.copy2(binary, executable)
            if digest(executable) != binary_hash:
                raise RuntimeError("fixture executable copy changed the binary")
        with service(executable, state, timeout, *flags) as (start, ready):
            response = call(state, "revision")
            done = time.monotonic_ns()
            if response != {
                "outcome": "revision",
                "revision": {"kind": "finite", "value": str(revision)},
            }:
                raise RuntimeError(f"unexpected fixture revision: {response}")
        rows.append((ready - start, done - ready, done - start))
    columns = ("readiness", "request_after_readiness", "first_reply")
    return {
        "profile": "new_binary_copy" if fresh_copy else mode,
        "samples_ns": [dict(zip(columns, row)) for row in rows],
        "median_ns": dict(zip(columns, map(statistics.median, zip(*rows)))),
        "maximum_ns": dict(zip(columns, map(max, zip(*rows)))),
    }


def digest(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def require_revision(state, expected):
    response = call(state, "revision")
    if response != {
        "outcome": "revision",
        "revision": {"kind": "finite", "value": str(expected)},
    }:
        raise RuntimeError(f"unexpected fixture revision: {response}")


def qualify_upgrade(previous, binary, state, timeout, flags, tasks):
    with service(previous, state, timeout, "--allow-writes"):
        revision = populate(state, tasks)
    legacy = {path: digest(path) for path in state.glob("checkpoint-*.bin")}
    if not legacy:
        raise RuntimeError("prior binary did not produce a legacy checkpoint")
    subprocess.run([binary, "--migrate-envelope", state], check=True,
                   timeout=timeout, capture_output=True)
    if any(digest(path) != expected for path, expected in legacy.items()):
        raise RuntimeError("migration changed a legacy checkpoint")
    with service(binary, state, timeout, *flags, "--allow-writes"):
        require_revision(state, revision)
        result = call(state, "enable", version=5,
                      expected={"kind": "finite", "value": str(revision)})
        if result != {"outcome": "control", "result": {"result": "applied"}}:
            raise RuntimeError(f"V5 fixture enablement refused: {result}")
    records = {path: digest(path) for path in state.glob("service-*.bin")}
    if not records:
        raise RuntimeError("current binary did not produce service records")
    rollback = subprocess.run([previous, state], timeout=timeout, capture_output=True)
    if rollback.returncode != 1 or b"ready" in rollback.stderr:
        raise RuntimeError(f"prior binary did not refuse newer state: {rollback.stderr!r}")
    if records != {path: digest(path) for path in state.glob("service-*.bin")}:
        raise RuntimeError("prior binary changed newer service records")
    with service(binary, state, timeout, *flags):
        require_revision(state, revision + 1)
    return {
        "previous_binary_sha256": digest(previous),
        "legacy_checkpoint_count": len(legacy),
        "migration_revision": revision,
        "recovered_revision": revision + 1,
        "downgrade_refusal": rollback.stderr.decode().strip(),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--trials", type=int, default=8)
    parser.add_argument("--tasks", type=int, default=1000)
    parser.add_argument("--readiness-timeout", type=float, default=30)
    parser.add_argument("--previous-binary", type=Path,
                        help="also qualify legacy migration and downgrade refusal")
    parser.add_argument("--fresh-copy", action="store_true",
                        help="launch a byte-identical temporary binary copy for each new store")
    args = parser.parse_args()
    if not 1 <= args.trials <= 100 or not 1 <= args.tasks <= 10000:
        parser.error("trials must be 1–100 and tasks must be 1–10000")
    if not 0 < args.readiness_timeout <= 300:
        parser.error("readiness timeout must be positive and at most 300 seconds")
    # Keep a caller-supplied alias in the actual launch path.
    binary = args.binary.absolute()
    flags = ("--atomic-state", "--allocation-receipts")
    report = {"binary_sha256": digest(binary), "fixture_tasks": args.tasks,
              "profiles": []}
    with tempfile.TemporaryDirectory(prefix="ch-r4-", dir=Path("/tmp").resolve()) as path:
        root = Path(path)
        stage = "new_binary_copy" if args.fresh_copy else "new"
        try:
            report["profiles"].append(measure(
                binary, root, "new", args.trials, args.readiness_timeout, flags, 0,
                fresh_copy=args.fresh_copy,
            ))
            stage = "reopen_empty"
            report["profiles"].append(measure(
                binary, root / "0", stage, args.trials, args.readiness_timeout, flags, 0,
            ))
            stage = "populate"
            populated = root / "populated"
            with service(binary, populated, args.readiness_timeout, *flags, "--allow-writes"):
                revision = populate(populated, args.tasks)
            stage = "reopen_populated"
            report["profiles"].append(measure(
                binary, populated, stage, args.trials, args.readiness_timeout, flags, revision,
            ))
            if args.previous_binary is not None:
                stage = "upgrade"
                report["upgrade"] = qualify_upgrade(
                    args.previous_binary.resolve(strict=True), binary, root / "upgrade",
                    args.readiness_timeout, flags, args.tasks,
                )
        except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
            report["failure"] = {
                "stage": stage,
                "readiness_timeout_seconds": args.readiness_timeout,
                "message": str(error),
            }
    print(json.dumps(report, indent=2))
    return 1 if "failure" in report else 0


if __name__ == "__main__":
    raise SystemExit(main())
