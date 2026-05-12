.PHONY: build check test clippy fmt lint docs-build docs-serve docs-clean run run-mock run-browser run-browser-headed clean all

# ── Cargo ──────────────────────────────────────────────────────────

build:
	cargo build

check:
	cargo check

test:
	cargo test --workspace

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all --check

lint: fmt clippy

all: fmt clippy test

clean:
	cargo clean

# ── Run ─────────────────────────────────────────────────────────────

run:
	cargo run -p reshape-cli -- --workspace ./page

run-mock:
	cargo run -p reshape-cli -- --workspace ./page --mock-llm

run-browser:
	cargo run -p reshape-cli -- --workspace ./page --render-browser

run-browser-headed:
	cargo run -p reshape-cli -- --workspace ./page --render-browser --browser-headed

# ── Docs (mdBook) ──────────────────────────────────────────────────

docs-build:
	mdbook build docs

docs-serve:
	mdbook serve docs --open

docs-clean:
	rm -rf docs/book

# ── Combined ────────────────────────────────────────────────────────

verify: lint test docs-build