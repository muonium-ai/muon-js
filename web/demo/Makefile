SHELL := /bin/sh

ROOT_DIR ?= ../..
RUSTUP ?= rustup
WASM_BINDGEN ?= wasm-bindgen
NPM ?= npm
STABLE_CARGO ?= $(shell $(RUSTUP) which cargo --toolchain stable)
STABLE_RUSTC ?= $(shell $(RUSTUP) which rustc --toolchain stable)

WASM_OUT ?= src/wasm
WASM_TARGET ?= $(ROOT_DIR)/target/wasm32-unknown-unknown/release/muon_js.wasm

.PHONY: wasm dev build test

wasm:
	@mkdir -p $(WASM_OUT)
	cd $(ROOT_DIR) && ./scripts/sync_version.sh
	cd $(ROOT_DIR) && RUSTC=$(STABLE_RUSTC) $(STABLE_CARGO) build --release --target wasm32-unknown-unknown --features mini-redis-wasm
	$(WASM_BINDGEN) --target web --out-dir $(WASM_OUT) $(WASM_TARGET)

dev: wasm
	$(NPM) install
	$(NPM) run dev

build: wasm
	$(NPM) install
	$(NPM) run build

test: wasm
	$(NPM) install
	$(NPM) run test
