SHELL := /bin/sh

CARGO ?= cargo
RUSTUP ?= rustup
WEB_DEMO_DIR ?= web/demo

.PHONY: build test release clean sync-version test-integration test-mquickjs test-mquickjs-detailed test-all js-runtime-bench js-runtime-bench-baseline js-runtime-bench-check muoncache muoncache-release muoncache-persist muoncache-persist-release muoncache-persist-release-bg muoncache-stop muoncache-parity muoncache-parity-verbose muoncache-runloop muoncache-benchmark muoncache-pipelined-benchmark redis-run redis-benchmark redis-pipelined-benchmark redis-stop redis-lua-tests redis-lua-benchmark muoncache-js-tests muoncache-js-tests-faithful redis-lua-scripting-bench muoncache-js-scripting-bench muoncache-js-scripting-bench-hotspots lua-js-perf-baseline lua-js-perf-check lua-js-mt-bench pipelined-benchmark-compare perf-benchmark perf-benchmark-no-redis web-demo-wasm web-demo-dev web-demo-build web-demo-test

MUON_CACHE_HOST ?= 127.0.0.1
MUON_CACHE_PORT ?= 6379
MUON_CACHE_PERSIST ?= tmp/muon_cache_$(shell date +%Y%m%d_%H%M%S).db
MUON_CACHE_PIDFILE ?= tmp/muon_cache.pid
MUON_CACHE_PORTFILE ?= tmp/muon_cache.port
MUON_CACHE_DBFILE ?= tmp/muon_cache.dbpath
MUON_CACHE_AOF ?= 0
MUON_CACHE_BENCH_LOG ?= tmp/muon_cache_benchmark_$(shell date +%Y%m%d_%H%M%S).log
REDIS_PORT ?= 6379
REDIS_PIDFILE ?= tmp/redis.pid
REDIS_LOG ?= tmp/redis.log
REDIS_BENCH_LOG ?= tmp/redis_benchmark_$(shell date +%Y%m%d_%H%M%S).log
REDIS_LUA_TEST_LOG ?= tmp/redis_lua_tests_$(shell date +%Y%m%d_%H%M%S).log
REDIS_LUA_SCRIPT_BENCH_LOG ?= tmp/redis_lua_script_bench_$(shell date +%Y%m%d_%H%M%S).log
MUON_CACHE_JS_TEST_LOG ?= tmp/muon_cache_js_tests_$(shell date +%Y%m%d_%H%M%S).log
MUON_CACHE_JS_FAITHFUL_TEST_LOG ?= tmp/muon_cache_js_faithful_tests_$(shell date +%Y%m%d_%H%M%S).log
MUON_CACHE_JS_FAITHFUL_BENCH_LOG ?= tmp/muon_cache_js_faithful_bench_$(shell date +%Y%m%d_%H%M%S).log
MUON_CACHE_JS_HOTSPOT_CASES ?= hash_sum set_members bulk_incr
MUON_CACHE_JS_HOTSPOT_ITERS ?= 1000
MUON_CACHE_JS_HOTSPOT_WARMUP ?= 200
MUON_CACHE_JS_HOTSPOT_JSON ?= tmp/muon_cache_js_hotspots_$(shell date +%Y%m%d_%H%M%S).json
MUON_CACHE_JS_HOTSPOT_CSV ?= tmp/muon_cache_js_hotspots_$(shell date +%Y%m%d_%H%M%S).csv
PIPELINE_DEPTH ?= 16
PIPELINE_REQUESTS ?= 200000
PIPELINE_TESTS ?= GET,SET,INCR,LPUSH,LPOP,RPUSH,RPOP,SADD,HSET
MUON_CACHE_PIPE_BENCH_LOG ?= tmp/muon_cache_pipelined_bench_$(shell date +%Y%m%d_%H%M%S).log
REDIS_PIPE_BENCH_LOG ?= tmp/redis_pipelined_bench_$(shell date +%Y%m%d_%H%M%S).log
LUA_JS_GATE_ROUNDS ?= 3
LUA_JS_GATE_REDIS_BASE_PORT ?= 6385
LUA_JS_GATE_OUT ?= tmp/comparison/lua_js_perf_gate_$(shell date +%Y%m%d_%H%M%S).json
LUA_JS_GATE_BASELINE ?= devdocs/lua_js_perf_baseline.json
LUA_JS_GATE_MAX_REGRESSION ?= 0.10
LUA_JS_GATE_CRITICAL_CASES ?= hash_sum set_members bulk_incr
LUA_JS_GATE_LOG_DIR ?= tmp/comparison/lua_js_gate
JS_BENCH_ITERS ?= 5000
JS_BENCH_WARMUP ?= 500
JS_BENCH_RUNS ?= 5
JS_BENCH_OUT ?= tmp/comparison/js_runtime_benchmark_$(shell date +%Y%m%d_%H%M%S).json
JS_BENCH_CHECK_ITERS ?= 2000
JS_BENCH_CHECK_WARMUP ?= 200
JS_BENCH_CHECK_RUNS ?= 3
JS_BENCH_BASELINE ?= devdocs/js_runtime_benchmark_baseline.json
JS_BENCH_CHECK_OUT ?= tmp/comparison/js_runtime_benchmark_check_$(shell date +%Y%m%d_%H%M%S).json
JS_BENCH_MAX_REGRESSION ?= 0.20
MT_BENCH_THREADS ?= 8
MT_BENCH_TOTAL ?= 1000000
MT_BENCH_WARMUP ?= 200
MT_BENCH_ROUNDS ?= 1
MT_BENCH_REDIS_BASE_PORT ?= 6390
MT_BENCH_LOG_DIR ?= tmp/comparison/mt_bench
MT_BENCH_OUT ?= tmp/comparison/mt_bench_$(shell date +%Y%m%d_%H%M%S).json
PERF_BENCH_MINI_PORT ?= 6380
PERF_BENCH_REDIS_PORT ?= 6379
PERF_BENCH_CLIENTS ?= 50
PERF_BENCH_REQUESTS ?= 1000000
PERF_BENCH_PIPELINE ?= 16
PERF_BENCH_RUNS ?= 5
PERF_BENCH_TESTS ?= get,set,incr,lpush,rpush,lpop,rpop,sadd,hset

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

web-demo-wasm: sync-version
	$(MAKE) -C $(WEB_DEMO_DIR) wasm

web-demo-dev: web-demo-wasm
	$(MAKE) -C $(WEB_DEMO_DIR) dev

web-demo-build: web-demo-wasm
	$(MAKE) -C $(WEB_DEMO_DIR) build

web-demo-test: web-demo-wasm
	$(MAKE) -C $(WEB_DEMO_DIR) test

js-runtime-bench: sync-version
	@mkdir -p tmp/comparison
	@echo "Running JS runtime microbenchmarks"
	@echo "Output: $(JS_BENCH_OUT)"
	$(CARGO) run --release --bin bench_runtime -- --iterations $(JS_BENCH_ITERS) --warmup $(JS_BENCH_WARMUP) --runs $(JS_BENCH_RUNS) --out $(JS_BENCH_OUT)

js-runtime-bench-baseline: sync-version
	@mkdir -p tmp/comparison devdocs
	@echo "Generating JS runtime benchmark baseline"
	@echo "Baseline: $(JS_BENCH_BASELINE)"
	$(CARGO) run --release --bin bench_runtime -- --iterations $(JS_BENCH_CHECK_ITERS) --warmup $(JS_BENCH_CHECK_WARMUP) --runs $(JS_BENCH_CHECK_RUNS) --out $(JS_BENCH_BASELINE)

js-runtime-bench-check: sync-version
	@mkdir -p tmp/comparison
	@echo "Running JS runtime benchmark regression check"
	@echo "Baseline: $(JS_BENCH_BASELINE)"
	@echo "Current : $(JS_BENCH_CHECK_OUT)"
	$(CARGO) run --release --bin bench_runtime -- --iterations $(JS_BENCH_CHECK_ITERS) --warmup $(JS_BENCH_CHECK_WARMUP) --runs $(JS_BENCH_CHECK_RUNS) --out $(JS_BENCH_CHECK_OUT)
	python3 tools/check_js_runtime_bench.py --baseline $(JS_BENCH_BASELINE) --current $(JS_BENCH_CHECK_OUT) --max-regression $(JS_BENCH_MAX_REGRESSION)

muoncache: sync-version
	@echo "Running muoncache on $(MUON_CACHE_HOST):$(MUON_CACHE_PORT)"
	$(CARGO) run --features muoncache --bin muon_cache -- --bind $(MUON_CACHE_HOST) --port $(MUON_CACHE_PORT)

muoncache-release: sync-version
	@echo "Running muoncache (release) on $(MUON_CACHE_HOST):$(MUON_CACHE_PORT)"
	$(CARGO) run --release --features muoncache --bin muon_cache -- --bind $(MUON_CACHE_HOST) --port $(MUON_CACHE_PORT)

muoncache-persist: sync-version
	@mkdir -p tmp
	@echo "Persist log file: $(MUON_CACHE_PERSIST)"
	@echo "Running muoncache with persistence at $(MUON_CACHE_PERSIST)"
	$(CARGO) run --features "muoncache muoncache-libsql" --bin muon_cache -- --bind $(MUON_CACHE_HOST) --port $(MUON_CACHE_PORT) --persist $(MUON_CACHE_PERSIST)

muoncache-persist-release: sync-version
	@mkdir -p tmp
	@echo "Persist log file: $(MUON_CACHE_PERSIST)"
	@port="$(MUON_CACHE_PORT)"; \
	if HOST="$(MUON_CACHE_HOST)" PORT="$$port" python3 -c 'import os,socket,sys; host=os.environ["HOST"]; port=int(os.environ["PORT"]); s=socket.socket(); s.settimeout(0.1); rc=s.connect_ex((host, port)); s.close(); sys.exit(0 if rc==0 else 1)'; then \
		port=$$(python3 scripts/pick_port.py); \
		echo "Port $(MUON_CACHE_PORT) in use; using $$port"; \
	fi; \
	echo "Running muoncache (release) with persistence at $(MUON_CACHE_PERSIST) on port $$port"; \
	aof_flag=""; \
	if [ "$(MUON_CACHE_AOF)" = "1" ]; then aof_flag="--aof"; fi; \
	$(CARGO) run --release --features "muoncache muoncache-libsql" --bin muon_cache -- --bind $(MUON_CACHE_HOST) --port $$port --persist $(MUON_CACHE_PERSIST) $$aof_flag

muoncache-persist-release-bg: sync-version
	@mkdir -p tmp
	@echo "Persist log file: $(MUON_CACHE_PERSIST)"
	@port="$(MUON_CACHE_PORT)"; \
	if HOST="$(MUON_CACHE_HOST)" PORT="$$port" python3 -c 'import os,socket,sys; host=os.environ["HOST"]; port=int(os.environ["PORT"]); s=socket.socket(); s.settimeout(0.1); rc=s.connect_ex((host, port)); s.close(); sys.exit(0 if rc==0 else 1)'; then \
		port=$$(python3 scripts/pick_port.py); \
		echo "Port $(MUON_CACHE_PORT) in use; using $$port"; \
	fi; \
	echo "Running muoncache (release, background) with persistence at $(MUON_CACHE_PERSIST) on port $$port"; \
	echo $$port > $(MUON_CACHE_PORTFILE); \
	echo $(MUON_CACHE_PERSIST) > $(MUON_CACHE_DBFILE); \
	$(CARGO) build --release --features "muoncache muoncache-libsql"; \
	aof_flag=""; \
	if [ "$(MUON_CACHE_AOF)" = "1" ]; then aof_flag="--aof"; fi; \
	target/release/muon_cache --bind $(MUON_CACHE_HOST) --port $$port --persist $(MUON_CACHE_PERSIST) $$aof_flag & \
	echo $$! > $(MUON_CACHE_PIDFILE)

muoncache-stop:
	@if [ ! -f "$(MUON_CACHE_PIDFILE)" ]; then echo "No PID file at $(MUON_CACHE_PIDFILE)"; exit 1; fi; \
	pid=$$(cat $(MUON_CACHE_PIDFILE)); \
	if [ -z "$$pid" ]; then echo "Empty PID file"; exit 1; fi; \
	echo "Stopping muoncache pid=$$pid"; \
	kill -INT $$pid 2>/dev/null || true; \
	for i in 1 2 3 4 5; do \
		if ! kill -0 $$pid 2>/dev/null; then break; fi; \
		sleep 0.2; \
	done; \
	if kill -0 $$pid 2>/dev/null; then \
		echo "Force stopping muoncache pid=$$pid"; \
		kill -KILL $$pid 2>/dev/null || true; \
	fi; \
	rm -f $(MUON_CACHE_PIDFILE)

muoncache-runloop: sync-version
	@set -e; \
	mkdir -p tmp; \
	if [ -f "$(MUON_CACHE_PIDFILE)" ]; then \
		pid=$$(cat $(MUON_CACHE_PIDFILE) 2>/dev/null || true); \
		if [ -n "$$pid" ] && kill -0 $$pid 2>/dev/null; then \
			echo "Existing muoncache pid=$$pid detected; stopping first"; \
			$(MAKE) -s muoncache-stop || true; \
		else \
			rm -f $(MUON_CACHE_PIDFILE); \
		fi; \
	fi; \
	if [ -e "$(MUON_CACHE_PERSIST)" ]; then \
		echo "Warning: persist file exists: $(MUON_CACHE_PERSIST)"; \
	fi; \
	touch "$(MUON_CACHE_PERSIST)" 2>/dev/null || { echo "Persist file not writable: $(MUON_CACHE_PERSIST)"; exit 1; }; \
	avail_kb=$$(df -Pk "$(MUON_CACHE_PERSIST)" | awk 'NR==2 {print $$4}'); \
	if [ -z "$$avail_kb" ] || [ "$$avail_kb" -lt 10240 ]; then \
		echo "Insufficient disk space for persistence (need >=10MB)"; \
		exit 1; \
	fi; \
	echo "=== start muoncache (release) ==="; \
	MUON_CACHE_PERSIST=$(MUON_CACHE_PERSIST) MUON_CACHE_AOF=1 $(MAKE) -s muoncache-persist-release-bg; \
	if [ ! -f "$(MUON_CACHE_PORTFILE)" ]; then \
		echo "Port file missing: $(MUON_CACHE_PORTFILE)"; \
		$(MAKE) -s muoncache-stop || true; \
		exit 1; \
	fi; \
	port=$$(cat $(MUON_CACHE_PORTFILE)); \
	retries=20; \
	while [ $$retries -gt 0 ]; do \
		if python3 -c 'import socket,sys; s=socket.socket(); s.settimeout(0.2); rc=s.connect_ex(("127.0.0.1", int("'"$$port"'"))); s.close(); sys.exit(0 if rc==0 else 1)'; then \
			break; \
		fi; \
		retries=$$((retries-1)); \
		sleep 0.2; \
	done; \
	if [ $$retries -eq 0 ]; then \
		echo "muoncache did not start on port $$port"; \
		$(MAKE) -s muoncache-stop || true; \
		exit 1; \
	fi; \
	echo "=== run python tests on $$port ==="; \
	python3 tests/muon_cache_parity.py $(MUON_CACHE_HOST) $$port --perf-retain; \
	echo "=== stop muoncache ==="; \
	$(MAKE) -s muoncache-stop; \
	if [ ! -f "$(MUON_CACHE_DBFILE)" ]; then \
		echo "DB path file missing: $(MUON_CACHE_DBFILE)"; \
		exit 1; \
	fi; \
	path=$$(cat $(MUON_CACHE_DBFILE)); \
	echo "=== persisted db: $$path ==="; \
	python3 scripts/read_muon_cache_db.py $$path; \
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

redis-pipelined-benchmark:
	@mkdir -p tmp
	@echo "Starting redis-server on port $(REDIS_PORT)"
	@$(MAKE) -s redis-run
	@echo "Running pipelined redis-benchmark -P $(PIPELINE_DEPTH) on port $(REDIS_PORT)"
	@echo "Benchmark log: $(REDIS_PIPE_BENCH_LOG)"
	@redis-benchmark -p $(REDIS_PORT) -t $(PIPELINE_TESTS) -P $(PIPELINE_DEPTH) -n $(PIPELINE_REQUESTS) | tee $(REDIS_PIPE_BENCH_LOG)
	@echo "Stopping redis"
	@$(MAKE) -s redis-stop

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

redis-lua-tests:
	@mkdir -p tmp
	@echo "Starting redis-server on port $(REDIS_PORT)"
	@$(MAKE) -s redis-run
	@echo "Running Lua scripting tests (log: $(REDIS_LUA_TEST_LOG))"
	@REDIS_HOST=$(MUON_CACHE_HOST) REDIS_PORT=$(REDIS_PORT) bash ./tests/scripting/run_lua_scripting_tests.sh 2>&1 | tee $(REDIS_LUA_TEST_LOG)
	@echo "Stopping redis"
	@$(MAKE) -s redis-stop

redis-lua-benchmark:
	@mkdir -p tmp
	@echo "Starting redis-server on port $(REDIS_PORT)"
	@$(MAKE) -s redis-run
	@echo "Running Lua scripting tests (log: $(REDIS_LUA_TEST_LOG))"
	@REDIS_HOST=$(MUON_CACHE_HOST) REDIS_PORT=$(REDIS_PORT) bash ./tests/scripting/run_lua_scripting_tests.sh 2>&1 | tee $(REDIS_LUA_TEST_LOG)
	@echo "Running redis-benchmark (log: $(REDIS_BENCH_LOG))"
	@$(MAKE) -s redis-benchmark
	@echo "Stopping redis"
	@$(MAKE) -s redis-stop

redis-lua-scripting-bench:
	@mkdir -p tmp
	@echo "Starting redis-server on port $(REDIS_PORT)"
	@$(MAKE) -s redis-run
	@echo "Running Redis Lua scripting benchmark (log: $(REDIS_LUA_SCRIPT_BENCH_LOG))"
	@python3 scripts/bench_scripting.py --host $(MUON_CACHE_HOST) --port $(REDIS_PORT) --suite tests/scripting/bench_suite.json | tee $(REDIS_LUA_SCRIPT_BENCH_LOG)
	@echo "Stopping redis"
	@$(MAKE) -s redis-stop

muoncache-js-tests: sync-version
	@set -e; \
	port=$$(python3 scripts/pick_port.py); \
	echo "Building muoncache (release)"; \
	$(CARGO) build --release --features muoncache --bin muon_cache; \
	echo "Starting muoncache (release) on $(MUON_CACHE_HOST):$$port"; \
	target/release/muon_cache --bind $(MUON_CACHE_HOST) --port $$port --script-mem 67108864 & \
	server_pid=$$!; \
	retries=80; \
	while [ $$retries -gt 0 ]; do \
		if python3 -c 'import socket,sys; s=socket.socket(); s.settimeout(0.2); rc=s.connect_ex(("$(MUON_CACHE_HOST)", int("'"$$port"'"))); s.close(); sys.exit(0 if rc==0 else 1)'; then \
			break; \
		fi; \
		retries=$$((retries-1)); \
		sleep 0.25; \
	done; \
	if [ $$retries -eq 0 ]; then \
		echo "muoncache did not start on port $$port"; \
		kill $$server_pid 2>/dev/null || true; \
		exit 1; \
	fi; \
	echo "Running muoncache JS scripting tests (log: $(MUON_CACHE_JS_TEST_LOG))"; \
	MUON_CACHE_HOST=$(MUON_CACHE_HOST) MUON_CACHE_PORT=$$port bash ./tests/scripting_js/run_js_scripting_tests.sh 2>&1 | tee $(MUON_CACHE_JS_TEST_LOG); \
	echo "Stopping muoncache"; \
	kill $$server_pid 2>/dev/null || true

muoncache-js-tests-faithful: sync-version
	@set -e; \
	port=$$(python3 scripts/pick_port.py); \
	echo "Building muoncache (release)"; \
	$(CARGO) build --release --features muoncache --bin muon_cache; \
	echo "Starting muoncache (release) on $(MUON_CACHE_HOST):$$port"; \
	target/release/muon_cache --bind $(MUON_CACHE_HOST) --port $$port & \
	server_pid=$$!; \
	retries=80; \
	while [ $$retries -gt 0 ]; do \
		if python3 -c 'import socket,sys; s=socket.socket(); s.settimeout(0.2); rc=s.connect_ex(("$(MUON_CACHE_HOST)", int("'"$$port"'"))); s.close(); sys.exit(0 if rc==0 else 1)'; then \
			break; \
		fi; \
		retries=$$((retries-1)); \
		sleep 0.25; \
	done; \
	if [ $$retries -eq 0 ]; then \
		echo "muoncache did not start on port $$port"; \
		kill $$server_pid 2>/dev/null || true; \
		exit 1; \
	fi; \
	echo "Running muoncache JS scripting tests (faithful) (log: $(MUON_CACHE_JS_FAITHFUL_TEST_LOG))"; \
	MUON_CACHE_HOST=$(MUON_CACHE_HOST) MUON_CACHE_PORT=$$port bash ./tests/scripting_js_faithful/run_js_scripting_tests.sh 2>&1 | tee $(MUON_CACHE_JS_FAITHFUL_TEST_LOG); \
	echo "Stopping muoncache"; \
	kill $$server_pid 2>/dev/null || true

muoncache-js-scripting-bench: sync-version
	@set -e; \
	port=$$(python3 scripts/pick_port.py); \
	echo "Building muoncache (release)"; \
	$(CARGO) build --release --features muoncache --bin muon_cache; \
	echo "Starting muoncache (release) on $(MUON_CACHE_HOST):$$port"; \
	target/release/muon_cache --bind $(MUON_CACHE_HOST) --port $$port & \
	server_pid=$$!; \
	retries=80; \
	while [ $$retries -gt 0 ]; do \
		if python3 -c 'import socket,sys; s=socket.socket(); s.settimeout(0.2); rc=s.connect_ex(("$(MUON_CACHE_HOST)", int("'"$$port"'"))); s.close(); sys.exit(0 if rc==0 else 1)'; then \
			break; \
		fi; \
		retries=$$((retries-1)); \
		sleep 0.25; \
	done; \
	if [ $$retries -eq 0 ]; then \
		echo "muoncache did not start on port $$port"; \
		kill $$server_pid 2>/dev/null || true; \
		exit 1; \
	fi; \
	echo "Running muoncache JS scripting benchmark (log: $(MUON_CACHE_JS_FAITHFUL_BENCH_LOG))"; \
	python3 scripts/bench_scripting.py --host $(MUON_CACHE_HOST) --port $$port --suite tests/scripting_js_faithful/bench_suite.json | tee $(MUON_CACHE_JS_FAITHFUL_BENCH_LOG); \
	echo "Stopping muoncache"; \
	kill $$server_pid 2>/dev/null || true

muoncache-js-scripting-bench-hotspots: sync-version
	@set -e; \
	port=$$(python3 scripts/pick_port.py); \
	echo "Building muoncache (release)"; \
	$(CARGO) build --release --features muoncache --bin muon_cache; \
	echo "Starting muoncache (release) on $(MUON_CACHE_HOST):$$port"; \
	target/release/muon_cache --bind $(MUON_CACHE_HOST) --port $$port & \
	server_pid=$$!; \
	retries=80; \
	while [ $$retries -gt 0 ]; do \
		if python3 -c 'import socket,sys; s=socket.socket(); s.settimeout(0.2); rc=s.connect_ex(("$(MUON_CACHE_HOST)", int("'"$$port"'"))); s.close(); sys.exit(0 if rc==0 else 1)'; then \
			break; \
		fi; \
		retries=$$((retries-1)); \
		sleep 0.25; \
	done; \
	if [ $$retries -eq 0 ]; then \
		echo "muoncache did not start on port $$port"; \
		kill $$server_pid 2>/dev/null || true; \
		exit 1; \
	fi; \
	echo "Running hotspot benchmark cases: $(MUON_CACHE_JS_HOTSPOT_CASES)"; \
	echo "JSON output: $(MUON_CACHE_JS_HOTSPOT_JSON)"; \
	echo "CSV output : $(MUON_CACHE_JS_HOTSPOT_CSV)"; \
	python3 scripts/bench_scripting.py --host $(MUON_CACHE_HOST) --port $$port --suite tests/scripting_js_faithful/bench_suite.json --iterations $(MUON_CACHE_JS_HOTSPOT_ITERS) --warmup $(MUON_CACHE_JS_HOTSPOT_WARMUP) --cases $(MUON_CACHE_JS_HOTSPOT_CASES) --out-json $(MUON_CACHE_JS_HOTSPOT_JSON) --out-csv $(MUON_CACHE_JS_HOTSPOT_CSV); \
	echo "Stopping muoncache"; \
	kill $$server_pid 2>/dev/null || true

lua-js-perf-baseline: sync-version
	@mkdir -p tmp/comparison devdocs
	@echo "Generating Lua-vs-JS performance baseline"
	@echo "Baseline JSON: $(LUA_JS_GATE_BASELINE)"
	python3 tools/lua_js_perf_gate.py --rounds $(LUA_JS_GATE_ROUNDS) --redis-base-port $(LUA_JS_GATE_REDIS_BASE_PORT) --log-dir $(LUA_JS_GATE_LOG_DIR) --out $(LUA_JS_GATE_BASELINE)

lua-js-perf-check: sync-version
	@mkdir -p tmp/comparison
	@echo "Running Lua-vs-JS performance regression check"
	@echo "Current output: $(LUA_JS_GATE_OUT)"
	@echo "Baseline      : $(LUA_JS_GATE_BASELINE)"
	python3 tools/lua_js_perf_gate.py --rounds $(LUA_JS_GATE_ROUNDS) --redis-base-port $(LUA_JS_GATE_REDIS_BASE_PORT) --log-dir $(LUA_JS_GATE_LOG_DIR) --out $(LUA_JS_GATE_OUT) --baseline $(LUA_JS_GATE_BASELINE) --max-regression $(LUA_JS_GATE_MAX_REGRESSION) --critical-cases $(LUA_JS_GATE_CRITICAL_CASES)

muoncache-parity: sync-version
	@port=$$(python3 scripts/pick_port.py); \
	echo "Starting muoncache and running parity checks"; \
	echo "Server: $(CARGO) run --features muoncache --bin muon_cache -- --bind $(MUON_CACHE_HOST) --port $$port"; \
	echo "Client: python3 tests/muon_cache_parity.py $(MUON_CACHE_HOST) $$port"; \
	set -e; \
	$(CARGO) run --features muoncache --bin muon_cache -- --bind $(MUON_CACHE_HOST) --port $$port & \
	server_pid=$$!; \
	sleep 0.5; \
	python3 tests/muon_cache_parity.py $(MUON_CACHE_HOST) $$port; \
	kill $$server_pid 2>/dev/null || true

muoncache-parity-verbose: sync-version
	@set -eux; \
	port=$$(python3 scripts/pick_port.py); \
	echo "Server: $(CARGO) run --features muoncache --bin muon_cache -- --bind $(MUON_CACHE_HOST) --port $$port"; \
	echo "Client: python3 tests/muon_cache_parity.py $(MUON_CACHE_HOST) $$port"; \
	$(CARGO) run --features muoncache --bin muon_cache -- --bind $(MUON_CACHE_HOST) --port $$port & \
	server_pid=$$!; \
	sleep 0.5; \
	python3 tests/muon_cache_parity.py $(MUON_CACHE_HOST) $$port; \
	kill $$server_pid 2>/dev/null || true

muoncache-pipelined-benchmark: sync-version
	@mkdir -p tmp
	@echo "=== start muoncache (no-persist + release) ==="; \
	$(MAKE) -s muoncache-persist-release-bg; \
	if [ ! -f "$(MUON_CACHE_PORTFILE)" ]; then \
		echo "Port file missing: $(MUON_CACHE_PORTFILE)"; \
		$(MAKE) -s muoncache-stop || true; \
		exit 1; \
	fi; \
	port=$$(cat $(MUON_CACHE_PORTFILE)); \
	retries=80; \
	while [ $$retries -gt 0 ]; do \
		python3 -c 'import socket,sys; s=socket.socket(); s.settimeout(0.2); rc=s.connect_ex(("127.0.0.1", int(sys.argv[1]))); s.close(); sys.exit(0 if rc==0 else 1)' $$port && break; \
		retries=$$((retries-1)); \
		sleep 0.25; \
	done; \
	if [ $$retries -eq 0 ]; then \
		echo "muoncache did not start on port $$port"; \
		$(MAKE) -s muoncache-stop || true; \
		exit 1; \
	fi; \
	echo "=== running pipelined benchmark -P $(PIPELINE_DEPTH) on $$port ==="; \
	echo "Benchmark log: $(MUON_CACHE_PIPE_BENCH_LOG)"; \
	redis-benchmark -p $$port -t $(PIPELINE_TESTS) -P $(PIPELINE_DEPTH) -n $(PIPELINE_REQUESTS) | tee $(MUON_CACHE_PIPE_BENCH_LOG); \
	echo "=== stop muoncache ==="; \
	$(MAKE) -s muoncache-stop || true

pipelined-benchmark-compare: sync-version
	@mkdir -p tmp
	@echo "=== Pipelined Benchmark Comparison (P=$(PIPELINE_DEPTH)) ==="
	@$(MAKE) -s muoncache-pipelined-benchmark
	@$(MAKE) -s redis-pipelined-benchmark
	@echo "=== Comparison ==="
	@python3 tools/compare_benchmarks.py --mini $(MUON_CACHE_PIPE_BENCH_LOG) --redis $(REDIS_PIPE_BENCH_LOG)

muoncache-benchmark: sync-version
	@mkdir -p tmp
	@echo "=== start muoncache (persist + release) ==="; \
	MUON_CACHE_PERSIST=$(MUON_CACHE_PERSIST) MUON_CACHE_AOF=1 $(MAKE) -s muoncache-persist-release-bg; \
	if [ ! -f "$(MUON_CACHE_PORTFILE)" ]; then \
		echo "Port file missing: $(MUON_CACHE_PORTFILE)"; \
		$(MAKE) -s muoncache-stop || true; \
		exit 1; \
	fi; \
	port=$$(cat $(MUON_CACHE_PORTFILE)); \
	retries=80; \
	while [ $$retries -gt 0 ]; do \
		python3 -c 'import socket,sys; s=socket.socket(); s.settimeout(0.2); rc=s.connect_ex(("127.0.0.1", int(sys.argv[1]))); s.close(); sys.exit(0 if rc==0 else 1)' $$port && break; \
		retries=$$((retries-1)); \
		sleep 0.25; \
	done; \
	if [ $$retries -eq 0 ]; then \
		echo "muoncache did not start on port $$port"; \
		$(MAKE) -s muoncache-stop || true; \
		exit 1; \
	fi; \
	echo "=== running benchmark on $$port ==="; \
	echo "Benchmark log: $(MUON_CACHE_BENCH_LOG)"; \
	python3 scripts/muon_cache_benchmark.py --host $(MUON_CACHE_HOST) --port $$port 2>&1 | tee $(MUON_CACHE_BENCH_LOG); \
	echo "=== stop muoncache ==="; \
	$(MAKE) -s muoncache-stop || true

# ── redis-benchmark pipelined performance gate ──────────────────────────────
# Starts muoncache, runs redis-benchmark with pipelining against it,
# and optionally compares against a running Redis instance.
#
# make perf-benchmark             – muoncache vs Redis (Redis must be running on PERF_BENCH_REDIS_PORT)
# make perf-benchmark-no-redis    – muoncache only (no Redis comparison)
perf-benchmark: release
	@mkdir -p tmp
	@echo "Running pipelined performance benchmark (muoncache vs Redis)"
	@./tests/run_perf_benchmark.sh \
		--mini-port $(PERF_BENCH_MINI_PORT) \
		--redis-port $(PERF_BENCH_REDIS_PORT) \
		--clients $(PERF_BENCH_CLIENTS) \
		--requests $(PERF_BENCH_REQUESTS) \
		--pipeline $(PERF_BENCH_PIPELINE) \
		--runs $(PERF_BENCH_RUNS) \
		--tests "$(PERF_BENCH_TESTS)"

perf-benchmark-no-redis: release
	@mkdir -p tmp
	@echo "Running pipelined performance benchmark (muoncache only)"
	@./tests/run_perf_benchmark.sh \
		--mini-port $(PERF_BENCH_MINI_PORT) \
		--clients $(PERF_BENCH_CLIENTS) \
		--requests $(PERF_BENCH_REQUESTS) \
		--pipeline $(PERF_BENCH_PIPELINE) \
		--runs $(PERF_BENCH_RUNS) \
		--tests "$(PERF_BENCH_TESTS)" \
		--no-redis

lua-js-mt-bench: sync-version
	@mkdir -p tmp/comparison
	@echo "Running multi-threaded Lua-vs-JS benchmark"
	@echo "Threads: $(MT_BENCH_THREADS)  Total/case: $(MT_BENCH_TOTAL)  Rounds: $(MT_BENCH_ROUNDS)"
	python3 tools/lua_js_mt_bench.py --rounds $(MT_BENCH_ROUNDS) --threads $(MT_BENCH_THREADS) --total $(MT_BENCH_TOTAL) --warmup $(MT_BENCH_WARMUP) --redis-base-port $(MT_BENCH_REDIS_BASE_PORT) --log-dir $(MT_BENCH_LOG_DIR) --out $(MT_BENCH_OUT)

clean:
	$(CARGO) clean
