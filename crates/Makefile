.DEFAULT_GOAL := help

SHELL=bash
BASE ?= main

.PHONY: fmt
fmt:  ## Run rustfmt and dprint
	cargo fmt --all

.PHONY: check
check:  ## Run cargo check with all features
	cargo check -p ih-muse --all-features

.PHONY: clippy
clippy:  ## Run clippy with all features
	cargo clippy --all-targets --all-features -- -W clippy::dbg_macro

.PHONY: clippy-default
clippy-default:  ## Run clippy with default features
	cargo clippy --all-targets -- -W clippy::dbg_macro

.PHONY: pre-commit
pre-commit: fmt clippy clippy-default  ## Run autoformatting and linting

.PHONY: test
test:  ## Run tests
	cargo test --all-features \
		-p ih-muse \
		-- \
		--test-threads=2

.PHONY: integration-tests
integration-tests:  ## Run integration tests
	cargo test --all-features --test it -p ih-muse

.PHONY: test-doc
test-doc:  ## Run doc examples
	cargo test --doc \
	    -p ih-muse-core

.PHONY: doctest
doctest:  ## Check that documentation builds
	cargo doc --no-deps -p ih-muse-proto
	cargo doc --no-deps -p ih-muse-core
	cargo doc --no-deps -p ih-muse-record
	cargo doc --no-deps -p ih-muse-client
	cargo doc --no-deps -p ih-muse
	cargo doc --no-deps -p ih-muse-cli

.PHONY: publish
publish:  ## Publish ih-muse crates
	cargo publish --allow-dirty -p ih-muse-proto
	cargo publish --allow-dirty -p ih-muse-core
	cargo publish --allow-dirty -p ih-muse-record
	cargo publish --allow-dirty -p ih-muse-client
	cargo publish --allow-dirty -p ih-muse
	cargo publish --allow-dirty -p ih-muse-cli

.PHONY: help
help:  ## Display this help screen
	@echo -e "\033[1mAvailable commands:\033[0m"
	@grep -E '^[a-z.A-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}' | sort
