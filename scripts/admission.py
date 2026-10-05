"""Local exact-HEAD gate and immutable independent-review admission (stdlib only)."""

import hashlib
import json
import re
import subprocess
import sys
import time
from pathlib import Path
from typing import cast

__all__: list[str] = []


def _sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _git(*args: str) -> str:
    return subprocess.check_output(["git", *args], text=True).strip()


def _snapshot() -> dict[str, str]:
    if _git("status", "--porcelain"):
        raise ValueError("a clean index and worktree are required")
    return {"head": _git("rev-parse", "HEAD"), "tree": _git("rev-parse", "HEAD^{tree}")}


def _read(path: Path) -> dict[str, object]:
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise TypeError("receipt must be a JSON object")
    return cast(dict[str, object], value)


def _write(path: Path, data: dict[str, object]) -> None:
    temporary = path.with_suffix(".new")
    temporary.write_text(json.dumps(data, indent=2) + "\n")
    temporary.replace(path)


def _gate(directory: Path, snapshot: dict[str, str]) -> dict[str, object]:
    gate = _read(directory / "gate.json")
    if any(gate.get(key) != value for key, value in snapshot.items()):
        raise ValueError("stale gate HEAD/tree")
    if gate.get("exit_code") != 0 or gate.get("command") != ["just", "full-check"]:
        raise ValueError("missing successful full gate")
    if _sha(directory / "gate.log") != gate.get("log_sha256"):
        raise ValueError("gate log integrity mismatch")
    return gate


def _review(report: Path, digest: str, snapshot: dict[str, str], gate_hash: str) -> None:
    if not re.fullmatch(r"[0-9a-f]{64}", digest) or _sha(report) != digest:
        raise ValueError("review report integrity mismatch")
    text = report.read_text()
    for label, value in [("HEAD", snapshot["head"]), ("TREE", snapshot["tree"]),
                         ("GATE_SHA256", gate_hash), ("VERDICT", "PASS")]:
        matches = re.findall(rf"^{label}: (.+)$", text, re.MULTILINE)
        if matches != [value]:
            raise ValueError(f"review requires exactly one matching {label}")
    if len(text.splitlines()) < 8:
        raise ValueError("review must be an actual report, not a verdict-only assertion")


def _validate(directory: Path, snapshot: dict[str, str]) -> None:
    _gate(directory, snapshot)
    review = _read(directory / "review.json")
    if any(review.get(key) != value for key, value in snapshot.items()):
        raise ValueError("stale review HEAD/tree")
    gate_hash = _sha(directory / "gate.json")
    if review.get("gate_sha256") != gate_hash:
        raise ValueError("review predates this gate")
    _review(Path(cast(str, review["report"])), cast(str, review["report_sha256"]), snapshot, gate_hash)


def _push_refs(text: str, head: str) -> None:
    lines = text.splitlines()
    if not lines or len(text) > 65536:
        raise ValueError("missing or oversized pre-push ref input")
    for line in lines:
        fields = line.split()
        if len(fields) != 4 or fields[1] != head:
            raise ValueError("only the reviewed current HEAD may be pushed")


def _main() -> None:
    snapshot = _snapshot()
    identity = hashlib.sha256(_git("rev-parse", "--show-toplevel").encode()).hexdigest()[:16]
    directory = Path.home() / "tmp" / f"skillcfg-admission-{identity}"
    action = sys.argv[1] if len(sys.argv) > 1 else "verify"
    if action == "gate":
        directory.mkdir(mode=0o700, parents=True, exist_ok=True)
        # Invalidate earlier evidence before starting, including an interrupted gate.
        (directory / "gate.json").unlink(missing_ok=True)
        (directory / "review.json").unlink(missing_ok=True)
        with (directory / "gate.log").open("w") as log:
            result = subprocess.run(["just", "full-check"], stdout=log, stderr=subprocess.STDOUT, check=False)
        if _snapshot() != snapshot:
            raise ValueError("HEAD/tree changed during gate")
        _write(directory / "gate.json", {**snapshot, "command": ["just", "full-check"],
               "exit_code": result.returncode, "log_sha256": _sha(directory / "gate.log"),
               "completed_ns": time.time_ns()})
        print(directory / "gate.json")
        if result.returncode:
            raise ValueError("full gate failed; see gate.log")
    elif action == "review" and len(sys.argv) == 4:
        gate = _gate(directory, snapshot)
        report = Path(sys.argv[2]).resolve(strict=True)
        digest = sys.argv[3]
        gate_hash = _sha(directory / "gate.json")
        _review(report, digest, snapshot, gate_hash)
        if report.stat().st_mtime_ns < cast(int, gate["completed_ns"]):
            raise ValueError("review report must be created after the gate")
        _write(directory / "review.json", {**snapshot, "gate_sha256": gate_hash,
               "report": str(report), "report_sha256": digest})
        _validate(directory, snapshot)
        print("review admitted for", snapshot["head"])
    elif action in ("verify", "pre-push") and len(sys.argv) <= 2:
        if action == "pre-push":
            _push_refs(sys.stdin.read(65537), snapshot["head"])
        _validate(directory, snapshot)
        print("gate/review verified for", snapshot["head"])
    else:
        raise ValueError("usage: admission.py gate | review REPORT SHA256 | verify | pre-push")


if __name__ == "__main__":
    try:
        _main()
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        sys.exit(f"admission: {error}")
