SHELL := /bin/sh

CARGO ?= cargo

.PHONY: build test release clean sync-version test-integration test-mquickjs test-mquickjs-detailed test-all mini-redis mini-redis-parity mini-redis-parity-verbose

MINI_REDIS_HOST ?= 127.0.0.1
MINI_REDIS_PORT ?= 6380

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

mini-redis: sync-version
	@echo "Running mini-redis on $(MINI_REDIS_HOST):$(MINI_REDIS_PORT)"
	$(CARGO) run --features mini-redis --bin mini_redis -- --bind $(MINI_REDIS_HOST) --port $(MINI_REDIS_PORT)

mini-redis-parity: sync-version
	@port=$$(python3 scripts/pick_port.py); \
	echo "Starting mini-redis and running parity checks"; \
	echo "Server: $(CARGO) run --features mini-redis --bin mini_redis -- --bind $(MINI_REDIS_HOST) --port $$port"; \
	echo "Client: python3 tests/mini_redis_parity.py $(MINI_REDIS_HOST) $$port"; \
	set -e; \
	$(CARGO) run --features mini-redis --bin mini_redis -- --bind $(MINI_REDIS_HOST) --port $$port & \
	server_pid=$$!; \
	sleep 0.5; \
	python3 tests/mini_redis_parity.py $(MINI_REDIS_HOST) $$port; \
	kill $$server_pid 2>/dev/null || true

mini-redis-parity-verbose: sync-version
	@set -eux; \
	port=$$(python3 scripts/pick_port.py); \
	echo "Server: $(CARGO) run --features mini-redis --bin mini_redis -- --bind $(MINI_REDIS_HOST) --port $$port"; \
	echo "Client: python3 tests/mini_redis_parity.py $(MINI_REDIS_HOST) $$port"; \
	$(CARGO) run --features mini-redis --bin mini_redis -- --bind $(MINI_REDIS_HOST) --port $$port & \
	server_pid=$$!; \
	sleep 0.5; \
	python3 tests/mini_redis_parity.py $(MINI_REDIS_HOST) $$port; \
	kill $$server_pid 2>/dev/null || true

clean:
	$(CARGO) clean
