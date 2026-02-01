SHELL := /bin/sh

CARGO ?= cargo

.PHONY: build test release clean sync-version test-integration test-mquickjs test-mquickjs-detailed test-all mini-redis mini-redis-release mini-redis-persist mini-redis-persist-release mini-redis-persist-release-bg mini-redis-stop mini-redis-parity mini-redis-parity-verbose mini-redis-runloop redis-run redis-benchmark redis-stop

MINI_REDIS_HOST ?= 127.0.0.1
MINI_REDIS_PORT ?= 6379
MINI_REDIS_PERSIST ?= tmp/mini_redis_$(shell date +%Y%m%d_%H%M%S).db
MINI_REDIS_PIDFILE ?= tmp/mini_redis.pid
MINI_REDIS_PORTFILE ?= tmp/mini_redis.port
MINI_REDIS_DBFILE ?= tmp/mini_redis.dbpath
MINI_REDIS_AOF ?= 0
REDIS_PORT ?= 6379
REDIS_PIDFILE ?= tmp/redis.pid
REDIS_LOG ?= tmp/redis.log
REDIS_BENCH_LOG ?= tmp/redis_benchmark_$(shell date +%Y%m%d_%H%M%S).log

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

mini-redis-release: sync-version
	@echo "Running mini-redis (release) on $(MINI_REDIS_HOST):$(MINI_REDIS_PORT)"
	$(CARGO) run --release --features mini-redis --bin mini_redis -- --bind $(MINI_REDIS_HOST) --port $(MINI_REDIS_PORT)

mini-redis-persist: sync-version
	@mkdir -p tmp
	@echo "Persist log file: $(MINI_REDIS_PERSIST)"
	@echo "Running mini-redis with persistence at $(MINI_REDIS_PERSIST)"
	$(CARGO) run --features "mini-redis mini-redis-libsql" --bin mini_redis -- --bind $(MINI_REDIS_HOST) --port $(MINI_REDIS_PORT) --persist $(MINI_REDIS_PERSIST)

mini-redis-persist-release: sync-version
	@mkdir -p tmp
	@echo "Persist log file: $(MINI_REDIS_PERSIST)"
	@port="$(MINI_REDIS_PORT)"; \
	if HOST="$(MINI_REDIS_HOST)" PORT="$$port" python3 -c 'import os,socket,sys; host=os.environ["HOST"]; port=int(os.environ["PORT"]); s=socket.socket(); s.settimeout(0.1); rc=s.connect_ex((host, port)); s.close(); sys.exit(0 if rc==0 else 1)'; then \
		port=$$(python3 scripts/pick_port.py); \
		echo "Port $(MINI_REDIS_PORT) in use; using $$port"; \
	fi; \
	echo "Running mini-redis (release) with persistence at $(MINI_REDIS_PERSIST) on port $$port"; \
	aof_flag=""; \
	if [ "$(MINI_REDIS_AOF)" = "1" ]; then aof_flag="--aof"; fi; \
	$(CARGO) run --release --features "mini-redis mini-redis-libsql" --bin mini_redis -- --bind $(MINI_REDIS_HOST) --port $$port --persist $(MINI_REDIS_PERSIST) $$aof_flag

mini-redis-persist-release-bg: sync-version
	@mkdir -p tmp
	@echo "Persist log file: $(MINI_REDIS_PERSIST)"
	@port="$(MINI_REDIS_PORT)"; \
	if HOST="$(MINI_REDIS_HOST)" PORT="$$port" python3 -c 'import os,socket,sys; host=os.environ["HOST"]; port=int(os.environ["PORT"]); s=socket.socket(); s.settimeout(0.1); rc=s.connect_ex((host, port)); s.close(); sys.exit(0 if rc==0 else 1)'; then \
		port=$$(python3 scripts/pick_port.py); \
		echo "Port $(MINI_REDIS_PORT) in use; using $$port"; \
	fi; \
	echo "Running mini-redis (release, background) with persistence at $(MINI_REDIS_PERSIST) on port $$port"; \
	echo $$port > $(MINI_REDIS_PORTFILE); \
	echo $(MINI_REDIS_PERSIST) > $(MINI_REDIS_DBFILE); \
	$(CARGO) build --release --features "mini-redis mini-redis-libsql"; \
	aof_flag=""; \
	if [ "$(MINI_REDIS_AOF)" = "1" ]; then aof_flag="--aof"; fi; \
	target/release/mini_redis --bind $(MINI_REDIS_HOST) --port $$port --persist $(MINI_REDIS_PERSIST) $$aof_flag & \
	echo $$! > $(MINI_REDIS_PIDFILE)

mini-redis-stop:
	@if [ ! -f "$(MINI_REDIS_PIDFILE)" ]; then echo "No PID file at $(MINI_REDIS_PIDFILE)"; exit 1; fi; \
	pid=$$(cat $(MINI_REDIS_PIDFILE)); \
	if [ -z "$$pid" ]; then echo "Empty PID file"; exit 1; fi; \
	echo "Stopping mini-redis pid=$$pid"; \
	kill -INT $$pid 2>/dev/null || true; \
	for i in 1 2 3 4 5; do \
		if ! kill -0 $$pid 2>/dev/null; then break; fi; \
		sleep 0.2; \
	done; \
	if kill -0 $$pid 2>/dev/null; then \
		echo "Force stopping mini-redis pid=$$pid"; \
		kill -KILL $$pid 2>/dev/null || true; \
	fi; \
	rm -f $(MINI_REDIS_PIDFILE)

mini-redis-runloop: sync-version
	@set -e; \
	mkdir -p tmp; \
	if [ -f "$(MINI_REDIS_PIDFILE)" ]; then \
		pid=$$(cat $(MINI_REDIS_PIDFILE) 2>/dev/null || true); \
		if [ -n "$$pid" ] && kill -0 $$pid 2>/dev/null; then \
			echo "Existing mini-redis pid=$$pid detected; stopping first"; \
			$(MAKE) -s mini-redis-stop || true; \
		else \
			rm -f $(MINI_REDIS_PIDFILE); \
		fi; \
	fi; \
	if [ -e "$(MINI_REDIS_PERSIST)" ]; then \
		echo "Warning: persist file exists: $(MINI_REDIS_PERSIST)"; \
	fi; \
	touch "$(MINI_REDIS_PERSIST)" 2>/dev/null || { echo "Persist file not writable: $(MINI_REDIS_PERSIST)"; exit 1; }; \
	avail_kb=$$(df -Pk "$(MINI_REDIS_PERSIST)" | awk 'NR==2 {print $$4}'); \
	if [ -z "$$avail_kb" ] || [ "$$avail_kb" -lt 10240 ]; then \
		echo "Insufficient disk space for persistence (need >=10MB)"; \
		exit 1; \
	fi; \
	echo "=== start mini-redis (release) ==="; \
	MINI_REDIS_PERSIST=$(MINI_REDIS_PERSIST) MINI_REDIS_AOF=1 $(MAKE) -s mini-redis-persist-release-bg; \
	if [ ! -f "$(MINI_REDIS_PORTFILE)" ]; then \
		echo "Port file missing: $(MINI_REDIS_PORTFILE)"; \
		$(MAKE) -s mini-redis-stop || true; \
		exit 1; \
	fi; \
	port=$$(cat $(MINI_REDIS_PORTFILE)); \
	retries=20; \
	while [ $$retries -gt 0 ]; do \
		if python3 -c 'import socket,sys; s=socket.socket(); s.settimeout(0.2); rc=s.connect_ex(("127.0.0.1", int("'"$$port"'"))); s.close(); sys.exit(0 if rc==0 else 1)'; then \
			break; \
		fi; \
		retries=$$((retries-1)); \
		sleep 0.2; \
	done; \
	if [ $$retries -eq 0 ]; then \
		echo "mini-redis did not start on port $$port"; \
		$(MAKE) -s mini-redis-stop || true; \
		exit 1; \
	fi; \
	echo "=== run python tests on $$port ==="; \
	python3 tests/mini_redis_parity.py $(MINI_REDIS_HOST) $$port --perf-retain; \
	echo "=== stop mini-redis ==="; \
	$(MAKE) -s mini-redis-stop; \
	if [ ! -f "$(MINI_REDIS_DBFILE)" ]; then \
		echo "DB path file missing: $(MINI_REDIS_DBFILE)"; \
		exit 1; \
	fi; \
	path=$$(cat $(MINI_REDIS_DBFILE)); \
	echo "=== persisted db: $$path ==="; \
	python3 scripts/read_mini_redis_db.py $$path; \
	echo "=== perf summary above ==="

redis-run:
	@mkdir -p tmp
	@echo "Starting redis-server on port $(REDIS_PORT)"
	@redis-server --port $(REDIS_PORT) --daemonize yes --pidfile $(REDIS_PIDFILE) --logfile $(REDIS_LOG)

redis-benchmark:
	@mkdir -p tmp
	@echo "Running redis-benchmark on port $(REDIS_PORT)"
	@echo "Benchmark log: $(REDIS_BENCH_LOG)"
	@redis-benchmark -p $(REDIS_PORT) | tee $(REDIS_BENCH_LOG)

redis-stop:
	@if [ ! -f "$(REDIS_PIDFILE)" ]; then echo "No PID file at $(REDIS_PIDFILE)"; exit 1; fi; \
	pid=$$(cat $(REDIS_PIDFILE)); \
	if [ -z "$$pid" ]; then echo "Empty PID file"; exit 1; fi; \
	echo "Stopping redis pid=$$pid"; \
	kill -TERM $$pid 2>/dev/null || true; \
	for i in 1 2 3 4 5; do \
		if ! kill -0 $$pid 2>/dev/null; then break; fi; \
		sleep 0.2; \
	done; \
	if kill -0 $$pid 2>/dev/null; then \
		echo "Force stopping redis pid=$$pid"; \
		kill -KILL $$pid 2>/dev/null || true; \
	fi; \
	rm -f $(REDIS_PIDFILE)

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
