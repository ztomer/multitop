.PHONY: help build test clippy fmt fmt-check coverage clean audit

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

clippy: ## Run clippy with warnings as errors
	cargo clippy --workspace --all-targets -- -D warnings

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
