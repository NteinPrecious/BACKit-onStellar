.PHONY: build test clean build-optimized wasm-size-check

build:
	cargo build --release --target wasm32-unknown-unknown

test:
	cargo test

clean:
	cargo clean

# Build, optimise and report every contract (size + SHA-256), failing over the 128 KiB limit.
build-optimized:
	bash scripts/build-optimized.sh

# Check the size of already-built WASM without rebuilding.
wasm-size-check:
	bash scripts/build-optimized.sh --check
