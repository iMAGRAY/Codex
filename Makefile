SHELL := bash

CODex_CARGO_BIN := codex-rs/target/release/codex-cli
CARGO_PROFILE := release
CARGO_TARGET := codex-cli
ARGS ?=

.PHONY: build start

build:
	cd codex-rs && cargo build --$(CARGO_PROFILE) -p $(CARGO_TARGET)

start: build
	@if [ -z "$$OPENAI_API_KEY" ]; then \
		echo "OPENAI_API_KEY environment variable must be set" >&2; \
		exit 2; \
	fi
	cd codex-rs && target/$(CARGO_PROFILE)/codex-cli $(ARGS)
