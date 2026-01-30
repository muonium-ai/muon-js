SHELL := /bin/sh

CARGO ?= cargo

.PHONY: build test release clean sync-version test-integration test-mquickjs test-mquickjs-detailed test-all

sync-version:
	./scripts/sync_version.sh

build: sync-version
	$(CARGO) build

test: sync-version
	$(CARGO) test

test-integration: release
	@echo "Running integration tests..."
	@./tests/run_integration.sh

test-mquickjs: release
	@echo "Running mquickjs compatibility tests..."
	@./tests/run_mquickjs_tests.sh

test-mquickjs-detailed: release
	@echo "Running detailed mquickjs compatibility check..."
	@./tests/check_mquickjs_compatibility.sh

test-all: test test-integration test-mquickjs-detailed

release: sync-version
	$(CARGO) build --release

clean:
	$(CARGO) clean
