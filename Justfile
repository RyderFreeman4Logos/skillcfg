set dotenv-load := false

default:
    @just --list

fmt:
    ionice -c3 cargo fmt --all -- --check

clippy:
    ionice -c3 cargo clippy --workspace --all-targets -- -D warnings

test:
    ionice -c3 cargo test --workspace

test-core:
    ionice -c3 cargo test -p skillcfg-core

check: fmt clippy test

get key:
    ionice -c3 cargo run -q -p skillcfg -- --config examples/config.toml get {{key}}
