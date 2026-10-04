# skillcfg

Agent-independent runtime configuration for skills. This first phase provides a small Rust library and a script-friendly `get` command backed by one shared TOML file.

## Quick start

```sh
just get demo.message
```

This builds and runs the CLI against [`examples/config.toml`](examples/config.toml), printing `hello from skillcfg` to stdout. Choose another config explicitly with `skillcfg --config PATH get demo.message`.

Configuration paths are selected in this order: `--config`, `SKILLCFG_CONFIG`, `$XDG_CONFIG_HOME/skillcfg/config.toml`, then `$HOME/.config/skillcfg/config.toml`. The file may be a symlink. It must contain `schema_version = 1`; keys are literal dotted paths such as `demo.message`.

`get` writes only the requested value and a newline to stdout. Errors go to stderr and return a non-zero status.

## Development

Install the local commit hook with `lefthook install`. Run the full local checks with:

```sh
just check
```
