SHELL := /bin/sh

WASM_BINDGEN ?= wasm-bindgen
PYTHON3      ?= python3
PORT         ?= 8888
DIST_DIR     ?= dist
RUSTUP       ?= rustup

# Use rustup-managed toolchain (has wasm32-unknown-unknown target),
# even when Homebrew rustc appears first on PATH.
STABLE_CARGO := $(shell $(RUSTUP) which cargo  --toolchain stable 2>/dev/null || echo cargo)
STABLE_RUSTC := $(shell $(RUSTUP) which rustc  --toolchain stable 2>/dev/null || echo rustc)

WASM_TARGET  := wasm32-unknown-unknown
# cdylib output lives in the trafficlab-local target/ tree
WASM_OUT_DEV     := target/$(WASM_TARGET)/debug/trafficlab.wasm
WASM_OUT_RELEASE := target/$(WASM_TARGET)/release/trafficlab.wasm

.PHONY: build build-release serve release clean

## build: Compile Rust → WASM (debug profile) then generate JS glue into pkg/.
build:
	@mkdir -p pkg
	RUSTC=$(STABLE_RUSTC) $(STABLE_CARGO) build --target $(WASM_TARGET) \
	    --manifest-path Cargo.toml
	$(WASM_BINDGEN) --target web --out-dir pkg $(WASM_OUT_DEV)

## build-release: Compile optimised WASM (lto + opt-level=z) then generate glue.
build-release:
	@mkdir -p pkg
	RUSTC=$(STABLE_RUSTC) $(STABLE_CARGO) build --release --target $(WASM_TARGET) \
	    --manifest-path Cargo.toml
	$(WASM_BINDGEN) --target web --out-dir pkg $(WASM_OUT_RELEASE)

## serve: Build (dev) then serve the demo on http://127.0.0.1:PORT
serve: build
	@echo ""
	@echo "  TrafficLab running at http://127.0.0.1:$(PORT)"
	@echo "  Press Ctrl-C to stop."
	@echo ""
	$(PYTHON3) -m http.server $(PORT) --directory .

## release: Build release WASM and copy all web assets to dist/.
release: build-release
	@mkdir -p $(DIST_DIR)/pkg
	cp index.html app.js $(DIST_DIR)/
	cp pkg/trafficlab.js pkg/trafficlab_bg.wasm $(DIST_DIR)/pkg/
	@echo "Release artifacts written to $(DIST_DIR)/"

## deploy: Build release and copy web assets to web/trafficlab/ for GitHub Pages.
## The Pages workflow (pages.yml) also runs this path directly.
deploy: build-release
	@mkdir -p ../web/trafficlab/pkg
	cp index.html app.js ../web/trafficlab/
	cp pkg/trafficlab.js pkg/trafficlab_bg.wasm ../web/trafficlab/pkg/
	@echo "TrafficLab assets written to web/trafficlab/ (served at /muon-js/trafficlab/)"

## clean: Remove wasm-bindgen output and release artifacts.
clean:
	rm -rf pkg/ $(DIST_DIR)/ ../web/trafficlab/
