# skillcfg

Agent-independent runtime configuration for skills. This first phase provides a small Rust library and a script-friendly `get` command backed by one shared TOML file.

## Quick start

```sh
just get demo.message
```

This builds and runs the CLI against [`examples/config.toml`](examples/config.toml), printing `hello from skillcfg` to stdout. Choose another config explicitly with `skillcfg --config PATH get demo.message`.

Configuration paths are selected in this order: `--config`, `SKILLCFG_CONFIG`, `$XDG_CONFIG_HOME/skillcfg/config.toml`, then `$HOME/.config/skillcfg/config.toml`. Unset environment variables fall through; a selected but empty `SKILLCFG_CONFIG` or `XDG_CONFIG_HOME` is an error, not a fallback. The file may be a symlink. It must contain integer `schema_version = 1`.

Keys such as `demo.message` are literal dotted paths: each nonempty segment contains only ASCII letters, digits, `_`, or `-`. Quoted segments, Unicode segments, and escaping dots are not supported; dots always traverse tables.

`get` writes strings verbatim, including their existing newlines; other values use TOML text (arrays/tables use inline TOML). The CLI adds one newline only when the rendered value does not already end with one. The core renderer adds none. Errors go to stderr and return a non-zero status; syntax/schema diagnostics retain path, location or type, not unrelated source values.

## Development

Install both local hooks with `lefthook install`. `just check` is the fast pre-commit fmt/clippy check. After committing, run the full gate once on a clean HEAD:

```sh
just gate
```

The gate executes fmt, clippy, workspace tests and receipt tests, recording the actual command, exit code, HEAD, tree and log hash under `~/tmp/skillcfg-admission-<checkout-id>/`. No generated evidence is tracked or stored in `.git`.

Next obtain a genuine independent review of that exact HEAD after the successful gate. Its Markdown report must include exactly one standalone line each: `HEAD: <full SHA>`, `TREE: <full tree SHA>`, `GATE_SHA256: <sha256 of gate.json>` and `VERDICT: PASS`, plus the scope, checks and findings. Pin the complete report digest, then admit it:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/admission.py review /absolute/review.md <report-sha256>
just verify-admission
```

Admission checks integrity and the pinned verdict; it does not perform or replace independent review. Do not manufacture a PASS report. The pre-push hook refuses missing, failed, modified or stale gate/review evidence, and never reruns the unchanged full gate. Changing HEAD/tree requires a new gate and review. It also rejects pushed objects other than the reviewed current HEAD (including ref deletions).
