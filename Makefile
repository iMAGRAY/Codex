# Unified build entrypoints for Codex CLI releases

.PHONY: build build-linux build-windows ensure-target

WORKSPACE_DIR := codex-rs
DIST_DIR := dist
BIN_NAME := codex

INSTALL_BIN_DIR ?= $(HOME)/.local/bin
INSTALL_BIN_NAME ?= magray

LINUX_TARGET := x86_64-unknown-linux-gnu
WINDOWS_TARGET := x86_64-pc-windows-gnu

# Guard against silent failures when rustup is missing
RUSTUP := $(shell command -v rustup 2>/dev/null)
CARGO := $(shell command -v cargo 2>/dev/null)
MINGW_CC := $(shell command -v x86_64-w64-mingw32-gcc 2>/dev/null)

ifeq ($(strip $(RUSTUP)),)
  $(error rustup is required but not found in PATH)
endif
ifeq ($(strip $(CARGO)),)
  $(error cargo is required but not found in PATH)
endif

build: build-linux build-windows
	@echo "Artifacts ready under $(DIST_DIR)/"

build-linux: ensure-linux-target
	@echo "[build-linux] compiling $(BIN_NAME) for $(LINUX_TARGET)"
	@cd $(WORKSPACE_DIR) && cargo build --release --locked --bin $(BIN_NAME) --target $(LINUX_TARGET)
	@mkdir -p $(DIST_DIR)
	@cp $(WORKSPACE_DIR)/target/$(LINUX_TARGET)/release/$(BIN_NAME) $(DIST_DIR)/$(BIN_NAME)-$(LINUX_TARGET)
	@echo "[build-linux] output => $(DIST_DIR)/$(BIN_NAME)-$(LINUX_TARGET)"

build-windows: ensure-windows-target
ifeq ($(strip $(MINGW_CC)),)
	@echo "[build-windows] x86_64-w64-mingw32-gcc not found. Install mingw-w64 to enable Windows cross-compilation." >&2
	@exit 1
endif
	@echo "[build-windows] compiling $(BIN_NAME) for $(WINDOWS_TARGET)"
	@cd $(WORKSPACE_DIR) && cargo build --release --locked --bin $(BIN_NAME) --target $(WINDOWS_TARGET)
	@mkdir -p $(DIST_DIR)
	@cp $(WORKSPACE_DIR)/target/$(WINDOWS_TARGET)/release/$(BIN_NAME).exe $(DIST_DIR)/$(BIN_NAME)-$(WINDOWS_TARGET).exe
	@echo "[build-windows] output => $(DIST_DIR)/$(BIN_NAME)-$(WINDOWS_TARGET).exe"

ensure-linux-target:
	@cd $(WORKSPACE_DIR) && rustup target add $(LINUX_TARGET) >/dev/null 2>&1 || true

ensure-windows-target:
	@cd $(WORKSPACE_DIR) && rustup target add $(WINDOWS_TARGET) >/dev/null 2>&1 || true

install-linux: build-linux
	@mkdir -p $(INSTALL_BIN_DIR)
	@cp $(DIST_DIR)/$(BIN_NAME)-$(LINUX_TARGET) $(INSTALL_BIN_DIR)/$(INSTALL_BIN_NAME)
	@chmod +x $(INSTALL_BIN_DIR)/$(INSTALL_BIN_NAME)
	@echo "[install-linux] installed $(INSTALL_BIN_NAME) => $(INSTALL_BIN_DIR)/$(INSTALL_BIN_NAME)"
	@if command -v $(INSTALL_BIN_NAME) >/dev/null 2>&1; \
		then echo "[install-linux] $(INSTALL_BIN_NAME) is available in PATH"; \
		else echo "[install-linux] add $(INSTALL_BIN_DIR) to PATH to use $(INSTALL_BIN_NAME)"; \
		fi
