#!/usr/bin/env bash
# Build every contract, optimise it, and report size and SHA-256 per WASM.
#
#   scripts/build-optimized.sh            build + optimise + report + size check
#   scripts/build-optimized.sh --check    only check sizes of already-built WASM
#
# MAX_WASM_BYTES overrides the size cap (default 131072: the Soroban limit of
# 128 KiB for an uploaded contract). The release profile in Cargo.toml already sets
# opt-level = "z", lto, codegen-units = 1, panic = "abort" and strip = "symbols";
# `stellar contract build` then runs the WASM through its own optimiser, so
# `stellar contract optimize` below normally changes nothing and is kept as a
# guard that any future un-optimised artefact is caught.
set -euo pipefail

cd "$(dirname "$0")/.."
MAX_WASM_BYTES="${MAX_WASM_BYTES:-131072}"
OUT_DIR="target/wasm32v1-none/release"
OPT_DIR="target/optimized"

size_of() { stat -c%s "$1" 2>/dev/null || stat -f%z "$1"; }
sha_of() { sha256sum "$1" 2>/dev/null | cut -d' ' -f1 || shasum -a 256 "$1" | cut -d' ' -f1; }

if [ "${1:-}" != "--check" ]; then
    echo "Building contracts (release, wasm32v1-none)..."
    stellar contract build
fi

mkdir -p "$OPT_DIR"
fail=0
printf '| Contract | Built (bytes) | Optimized (bytes) | SHA-256 (optimized) |\n'
printf '| --- | ---: | ---: | --- |\n'
for wasm in "$OUT_DIR"/*.wasm; do
    [ -e "$wasm" ] || { echo "No WASM found in $OUT_DIR" >&2; exit 1; }
    name="$(basename "$wasm" .wasm)"
    built="$(size_of "$wasm")"
    if [ "${1:-}" = "--check" ]; then
        optimized="$built"
        hash="$(sha_of "$wasm")"
    else
        stellar contract optimize --wasm "$wasm" --wasm-out "$OPT_DIR/$name.wasm" >/dev/null 2>&1
        optimized="$(size_of "$OPT_DIR/$name.wasm")"
        hash="$(sha_of "$OPT_DIR/$name.wasm")"
    fi
    printf '| %s | %s | %s | `%s` |\n' "$name" "$built" "$optimized" "$hash"
    if [ "$optimized" -gt "$MAX_WASM_BYTES" ]; then
        echo "ERROR: $name is $optimized bytes, over the $MAX_WASM_BYTES byte limit" >&2
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "All contracts are within the $MAX_WASM_BYTES byte limit."
