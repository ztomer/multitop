.PHONY: help build test clippy clippy-host clippy-targets fmt fmt-check coverage clean audit

help: ## Show available commands
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2}'

build: ## Build release binary
	./build.sh

build-debug: ## Build debug binary
	./build.sh --debug

test: ## Run all tests (excluding ignored)
	cargo test --workspace

test-all: ## Run all tests including ignored
	cargo test --workspace -- --include-ignored

clippy: clippy-host clippy-targets ## Clippy with warnings as errors, every shipped target

clippy-host: ## Clippy for this machine's target (character for character what CI runs)
	cargo clippy --workspace --all-targets --all-features -- -D warnings

# THE AGENT IS HALF `cfg(target_os = ...)`, AND NO SINGLE HOST LINTS BOTH HALVES.
#
# `clippy-host` only ever checks the half that matches the machine it runs on.
# CI runs on ubuntu, so CI checks the Linux half; a developer on a Mac checks
# the macOS half. Between them the two halves look covered, but neither run
# fails on the other's code, and nothing compares them. Turning the workspace
# lints on for `crates/agent` (which had never opted in at all) found ten
# findings in the Linux half plus two `#[expect]`s that were unfulfilled there
# -- and an unfulfilled expectation under `-D warnings` is an error, so the
# Linux build was broken while every gate was green.
#
# So name the targets the code actually ships to and lint all of them from
# wherever this runs. A missing target is a hard failure with the command to
# fix it, never a skip: a gate that quietly inspects nothing passes exactly
# like a clean tree.
SHIPPED_TARGETS = x86_64-unknown-linux-musl aarch64-unknown-linux-musl aarch64-apple-darwin

clippy-targets: ## Clippy the agent for every target it is deployed to
	@installed="$$(rustup target list --installed)"; \
	for t in $(SHIPPED_TARGETS); do \
		if ! echo "$$installed" | grep -qx "$$t"; then \
			echo "  target $$t is not installed; this gate would inspect nothing."; \
			echo "  rustup target add $$t"; \
			exit 1; \
		fi; \
		echo "  clippy: $$t"; \
		cargo clippy -p multitop-agent --all-targets --all-features \
			--target "$$t" -- -D warnings || exit 1; \
	done

fmt: ## Format code with rustfmt
	cargo fmt --all

fmt-check: ## Check formatting without modifying
	cargo fmt --all -- --check

coverage: ## Generate coverage report (requires cargo-llvm-cov)
	cargo llvm-cov --workspace --html --open

coverage-check: ## Check coverage threshold (fails under 80%)
	cargo llvm-cov --workspace --fail-under-lines 80

audit: ## Run cargo audit for security advisories
	cargo audit

clean: ## Remove build artifacts
	cargo clean
	rm -rf target/
