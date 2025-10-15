SHELL := bash

CARGO_BIN := codex
CARGO_PROFILE := release
CARGO_TARGET := codex-cli
ARGS ?=

.PHONY: build start

build:
	cd codex-rs && cargo build --$(CARGO_PROFILE) -p $(CARGO_TARGET)

start: build
	cd codex-rs && target/$(CARGO_PROFILE)/$(CARGO_BIN) $(ARGS)
