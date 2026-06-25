.PHONY: build release test test-e2e lint clean run run-release _xferrum _xferrum-release

## Build the ferrum binary (debug) via xferrum
build: _xferrum
	./target/debug/xferrum build --output ./target/debug/ferrum

## Build the ferrum binary (release) via xferrum
release: _xferrum-release
	./target/release/xferrum build --release --output ./target/release/ferrum

## Build the ferrum-graph config visualiser
build-graph:
	cargo build -p ferrum-graph

## Generate a Graphviz DOT file from ferrum.toml and render it to ferrum.png
graph: build-graph
	./target/debug/ferrum-graph ferrum.toml | dot -Tpng -o ferrum.png

## Run the full unit/integration test suite (excludes e2e)
test:
	cargo test --workspace --exclude ferrum-e2e

## Run e2e integration tests against a real ferrum binary
test-e2e: build
	FERRUM_BIN=$(shell pwd)/target/debug/ferrum cargo test -p ferrum-e2e

## Run clippy across the workspace
lint:
	cargo clippy --workspace -- -D warnings

## Remove build artefacts
clean:
	cargo clean

## Start Ferrum on :8080 using ferrum.toml (must exist)
run: build
	FERRUM_CONFIG=ferrum.toml ./target/debug/ferrum

## Start Ferrum in release mode
run-release: release
	FERRUM_CONFIG=ferrum.toml ./target/release/ferrum

ferrum.png: build-graph ferrum.toml
	target/debug/ferrum-graph ferrum.toml | dot -T png > ferrum.png

# Internal targets
_xferrum:
	cargo build -p xferrum

_xferrum-release:
	cargo build -p xferrum --release
