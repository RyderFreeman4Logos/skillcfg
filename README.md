# skillcfg

Agent-independent runtime preferences for skills. A small headless Rust core discovers consumers, resolves shared TOML values, validates static dependencies and reports impact; the CLI never executes skill behavior.

## Quick start

```sh
just build
export PATH="$PWD/target/debug:$PATH"
export SKILLCFG_CONFIG="$PWD/examples/config.toml"
skillcfg get demo.message
skillcfg get-many demo.attempts demo.enabled --format json
skillcfg discover --verbose
skillcfg show-skill report
skillcfg explain demo.timeout
skillcfg validate --strict
skillcfg validate-skill examples/skills/report --strict
examples/skills/report/scripts/report 'sample report'
```

This builds the CLI and runs all seven commands against [`examples/config.toml`](examples/config.toml) and one portable skill-owned script. `get` prints `hello from skillcfg`; the script prints `sample report` under a centrally supplied deadline without exposing that deadline first. Change the one central value to affect every referencing skill. Choose another config explicitly with `skillcfg --config PATH get demo.message`.

Configuration paths are selected in this order: `--config`, `SKILLCFG_CONFIG`, `$XDG_CONFIG_HOME/skillcfg/config.toml`, then `$HOME/.config/skillcfg/config.toml`. Unset environment variables fall through; a selected but empty `SKILLCFG_CONFIG` or `XDG_CONFIG_HOME` is an error, not a fallback. The file may be a symlink to a regular UTF-8 file; directories/devices/FIFOs are rejected. It must contain integer `schema_version = 1`.

Keys such as `demo.message` are literal dotted paths: each nonempty segment contains only ASCII letters, digits, `_`, or `-`. Quoted segments, Unicode segments, and escaping dots are not supported; dots always traverse tables.

`get` writes strings verbatim, including their existing newlines; other values use TOML text (arrays/tables use inline TOML). The CLI adds one newline only when the rendered value does not already end with one. The core renderer adds none. Errors go to stderr and return a non-zero status; syntax/schema diagnostics retain path, location or type, not unrelated source values.

## Discovery

`skillcfg discover [--root PATH]... [--verbose]` lists logical names, sorted by name then canonical directory. Verbose output is one tab-separated row per skill: name, debug-quoted canonical path, debug-quoted exposure list, SKILL.md path, optional manifest path and per-exposure symlink status. Aliases of one canonical directory merge; distinct directories with the same name fail with both paths/exposures on stderr.

Optional `[discovery] roots = ["~/skills", "relative/root"]` selects roots in the shared config. Relative paths use the config's lexical parent, not the process cwd. Explicit CLI roots replace configured roots. Without configured roots, only `~/.{codex,hermes,claude,agents}/skills` are searched; missing conventional roots are silent, missing explicit roots fail. Discovery can run without a config file.

Directories containing the exact filename `SKILL.md` are candidates, including roots and nested skills. Plain or single/double-quoted frontmatter `name:` takes precedence over the canonical directory basename; names contain ASCII letters/digits/`_`/`-`/`.`. This is not a complete YAML parser. Symlinked roots, categories, skills and Markdown files are followed, including targets outside roots. Broken links/cycles warn and traversal continues; name/read errors fail. Active canonical ancestors stop cycles, with a 256-directory depth ceiling. `.git`, `target`, `node_modules`, `.cache`, and `__pycache__` are skipped. `[discovery] ignore = ["cache*", "scratch?"]` adds basename patterns (`*` and `?` only).

## Manifest and batch reads

A skill's optional `skillcfg.toml` declares dependencies, not copied values:

```toml
schema_version = 1
[visible]
style = "review.style"
[opaque]
model = "model_tiers.review.model_id"
```

`skillcfg show-skill review` resolves only visible bindings, sorted by alias. `--all` explicitly includes opaque values; `--format json` emits a compact JSON object. Missing optional manifests yield no bindings. Malformed manifests, non-string/invalid keys, duplicate aliases (including across both classes), and missing selected global keys fail before any stdout is written. Manifest symlinks are supported; broken ones are not treated as absent.

`skillcfg get-many key.one key.two [--format kv|json]` resolves all requested keys before writing and preserves input order; duplicates fail. Default KV has one `key=value` line per binding. Plain, unambiguous strings are unquoted; empty strings, leading/trailing whitespace, control characters, JSON-looking strings, boolean/null spellings and numeric strings are JSON-quoted. Numbers/bools use natural text; arrays/tables use compact JSON. JSON preserves TOML value types (datetimes become strings); non-finite floats are rejected in either batch mode. Treat KV as data, never `eval` it. Explicit `get` and `get-many` intentionally disclose requested values for scripts; `show-skill` never includes opaque values by default. This is preference storage, not a secret manager.

## Validation

`skillcfg validate [--root PATH]... [--strict]` checks the config, discovered skills, manifests and scripts. `skillcfg validate-skill PATH [--strict]` checks just that skill without traversing configured roots. Successful validation prints `ok`; errors leave stdout empty, report path/line and key on stderr, and exit 1. Usage errors exit 2. Warnings do not fail unless `--strict` is requested.

Checks include missing manifest keys, duplicate aliases, collisions, broken links, literal script key existence, undeclared script keys, statically unused opaque dependencies and non-executable shebang scripts. Visible dependencies need not appear in scripts. The read-only scanner recognizes line-local `skillcfg get literal.key`, quoted literal keys, and shell command substitutions. Comments and ordinary quoted strings are skipped. Dynamic keys, unsupported invocation syntax and options are reported as unverifiable, never guessed or executed. Here-document bodies are skipped and their declarations marked unknown. This is not a Bash parser, execution tracer or workflow validator; complex/multiline shell scripts still require skill-owned tests. Error output never includes resolved values or source excerpts.

## Impact inspection

`skillcfg explain KEY [--root PATH]...` intentionally shows the requested value, lexical and resolved config source paths, and sorted manifest/script reference locations grouped by logical skill. No other values are printed. `references: none` means no known consumer; dynamic invocations appear separately as `unknown`, not matches. The index is built in memory per invocation, with no daemon/cache or overrides. Malformed dependencies/read errors fail rather than claiming complete impact from a partial index.

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
