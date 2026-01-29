SHELL := /bin/sh

CARGO ?= cargo

.PHONY: build test release clean sync-version

sync-version:
	./scripts/sync_version.sh

build: sync-version
	$(CARGO) build

test: sync-version
	$(CARGO) test

release: sync-version
	$(CARGO) build --release

clean:
	$(CARGO) clean
