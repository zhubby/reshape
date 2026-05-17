.PHONY: build check test clippy fmt lint extension-build extension-package extension-reinstall run-browser-reinstall docs-build docs-serve docs-clean run run-mock run-browser run-browser-headed clean all

WORKSPACE ?= ./page
BROWSER_SESSION ?= reshape-main

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

# ── Browser Extension ───────────────────────────────────────────────

extension-build:
	cargo test -p reshape-cli --test ts_bindings
	cd extensions/reshape && npm run build

extension-package: extension-build
	cd extensions/reshape && npm run package

extension-reinstall: extension-build
	-cargo run -q -p agent-browser -- --session $(BROWSER_SESSION) close
	@socket_dir="$${AGENT_BROWSER_SOCKET_DIR:-$${XDG_RUNTIME_DIR:+$${XDG_RUNTIME_DIR}/agent-browser}}"; \
	if [ -z "$$socket_dir" ]; then socket_dir="$${HOME}/.agent-browser"; fi; \
	rm -f "$$socket_dir/$(BROWSER_SESSION).pid" \
		"$$socket_dir/$(BROWSER_SESSION).sock" \
		"$$socket_dir/$(BROWSER_SESSION).port" \
		"$$socket_dir/$(BROWSER_SESSION).stream" \
		"$$socket_dir/$(BROWSER_SESSION).engine" \
		"$$socket_dir/$(BROWSER_SESSION).provider" \
		"$$socket_dir/$(BROWSER_SESSION).extensions" \
		"$$socket_dir/$(BROWSER_SESSION).version"

# ── Run ─────────────────────────────────────────────────────────────

run:
	cargo run -p reshape-cli -- --workspace $(WORKSPACE)

run-mock:
	cargo run -p reshape-cli -- --workspace $(WORKSPACE) --mock-llm

run-browser:
	cargo run -p reshape-cli -- --workspace $(WORKSPACE) --render-browser

run-browser-headed:
	cargo run -p reshape-cli -- --workspace $(WORKSPACE) --render-browser --browser-headed

run-browser-reinstall: extension-reinstall
	cargo run -p reshape-cli -- --workspace $(WORKSPACE) --browser-session $(BROWSER_SESSION)

# ── Docs (mdBook) ──────────────────────────────────────────────────

docs-build:
	mdbook build docs

docs-serve:
	mdbook serve docs --open

docs-clean:
	rm -rf docs/book

# ── Combined ────────────────────────────────────────────────────────

verify: lint test docs-build
