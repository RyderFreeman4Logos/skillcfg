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

check: fmt clippy

test-admission:
    PYTHONDONTWRITEBYTECODE=1 python3 scripts/test_admission.py

full-check: check test test-admission

gate:
    PYTHONDONTWRITEBYTECODE=1 python3 scripts/admission.py gate

verify-admission:
    PYTHONDONTWRITEBYTECODE=1 python3 scripts/admission.py verify

get key:
    ionice -c3 cargo run -q -p skillcfg -- --config examples/config.toml get {{key}}
