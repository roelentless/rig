INSTALL_DIR  := $(HOME)/.local/bin
INSTALL_PATH := $(INSTALL_DIR)/rig

.PHONY: build install test fmt check

## build: compile debug binary
build:
	cargo build

## install: build release binary and replace local rig
install:
	cargo build --release
	@mkdir -p $(INSTALL_DIR)
	cp target/release/rig $(INSTALL_PATH)
	@echo "→ $(INSTALL_PATH)"

## test: run test suite
test:
	cargo test -- --test-threads=1

## fmt: format source code
fmt:
	cargo fmt

## check: run fmt check and tests
check:
	cargo fmt --check
	cargo test -- --test-threads=1
