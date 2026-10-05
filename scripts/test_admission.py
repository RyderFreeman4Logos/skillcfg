"""Synthetic receipt checks; no Git writes, builds, or real approval records."""

import json
import tempfile
from pathlib import Path

import admission as a

__all__: list[str] = []


def _main() -> None:
    snapshot = {"head": "1" * 40, "tree": "2" * 40}
    passed = 0
    with tempfile.TemporaryDirectory(prefix="skillcfg-admission-", dir=Path.home() / "tmp") as temp:
        root = Path(temp)
        report = root / "synthetic-review.md"

        def seed() -> None:
            (root / "gate.log").write_text("synthetic fixture, not a real gate\n")
            a._write(root / "gate.json", {**snapshot, "command": ["just", "full-check"],
                     "exit_code": 0, "log_sha256": a._sha(root / "gate.log")})
            report.write_text("# Synthetic test only\n\nScope: isolated fixture\nChecks: fixture\n"
                              f"HEAD: {snapshot['head']}\nTREE: {snapshot['tree']}\n"
                              f"GATE_SHA256: {a._sha(root / 'gate.json')}\nVERDICT: PASS\n")
            a._write(root / "review.json", {**snapshot, "gate_sha256": a._sha(root / "gate.json"),
                     "report": str(report), "report_sha256": a._sha(report)})

        def rejected(label: str) -> None:
            nonlocal passed
            try:
                a._validate(root, snapshot)
            except (OSError, ValueError, KeyError, TypeError):
                passed += 1
                print("PASS reject", label)
            else:
                raise AssertionError(label)

        rejected("absent gate")
        seed()
        (root / "review.json").unlink()
        rejected("absent review")
        for record, key, value in [("gate", "exit_code", 1), ("gate", "head", "3" * 40),
                                   ("gate", "tree", "3" * 40), ("review", "head", "3" * 40),
                                   ("review", "tree", "3" * 40), ("review", "gate_sha256", "0" * 64)]:
            seed()
            path = root / f"{record}.json"
            data = json.loads(path.read_text())
            data[key] = value
            a._write(path, data)
            rejected(f"{record} {key}")
        seed()
        report.write_text(report.read_text().replace("VERDICT: PASS", "VERDICT: FAIL"))
        data = json.loads((root / "review.json").read_text())
        data["report_sha256"] = a._sha(report)
        a._write(root / "review.json", data)
        rejected("actual FAIL verdict with matching digest")
        seed()
        report.write_text(report.read_text().replace(snapshot["head"], "3" * 40))
        data = json.loads((root / "review.json").read_text())
        data["report_sha256"] = a._sha(report)
        a._write(root / "review.json", data)
        rejected("report HEAD differs with matching digest")
        seed()
        report.write_text(report.read_text() + "tampering\n")
        rejected("changed report")
        seed()
        (root / "gate.log").write_text("tampering\n")
        rejected("changed gate log")
        seed()
        a._validate(root, snapshot)
        passed += 1
        print("PASS valid bound synthetic pair")
    assert passed == 13, passed
    a._push_refs(f"refs/heads/feature {snapshot['head']} refs/heads/feature {'0' * 40}\n", snapshot["head"])
    for text in ["", "bad", f"refs/heads/other {'3' * 40} refs/heads/other {'0' * 40}"]:
        try:
            a._push_refs(text, snapshot["head"])
        except ValueError:
            pass
        else:
            raise AssertionError("unreviewed pushed ref")
    print(f"{passed} admission checks passed (synthetic only)")
    print("4 pre-push ref checks passed")


if __name__ == "__main__":
    _main()
