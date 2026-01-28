SHELL := /bin/sh

CARGO ?= cargo

.PHONY: build test release clean

build:
	$(CARGO) build

test:
	$(CARGO) test

release:
	$(CARGO) build --release

clean:
	$(CARGO) clean
