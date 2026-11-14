#!/bin/sh
# Builds the release client into OUT_DIR (default dist/): the wasm32 build,
# wasm-bindgen (`--target web`), wasm-opt, and the loader. The one recipe
# for every build: this repo's CI, agora's Docker stage and `just web-build`.
# Usage: scripts/build.sh [OUT_DIR]
# Install the tools first with scripts/install-tools.sh.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
. "$ROOT/scripts/tools.sh"
OUT_DIR=${1:-$ROOT/dist}
TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target}

WASM_BINDGEN_VERSION=$(lock_version wasm-bindgen)
if [ "$(wasm-bindgen --version 2>/dev/null)" != "wasm-bindgen $WASM_BINDGEN_VERSION" ]; then
    echo "error: need wasm-bindgen $WASM_BINDGEN_VERSION (Cargo.lock); run scripts/install-tools.sh" >&2
    exit 1
fi
if ! wasm-opt --version 2>/dev/null | grep -q "^wasm-opt version $BINARYEN_VERSION "; then
    echo "error: need wasm-opt $BINARYEN_VERSION; run scripts/install-tools.sh" >&2
    exit 1
fi

cd "$ROOT"
cargo build --release --locked --target wasm32-unknown-unknown -p agora-web

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"
wasm-bindgen --target web --no-typescript --out-name agora-web --out-dir "$OUT_DIR" \
    "$TARGET_DIR/wasm32-unknown-unknown/release/agora-web.wasm"
# The features rustc enables by default for wasm32-unknown-unknown (Rust
# 1.82+); wasm-opt refuses input that uses features it wasn't given. Every
# current browser supports them.
wasm-opt -Oz --strip-debug --strip-producers \
    --enable-bulk-memory --enable-bulk-memory-opt --enable-multivalue \
    --enable-mutable-globals --enable-nontrapping-float-to-int \
    --enable-reference-types --enable-sign-ext \
    -o "$OUT_DIR/agora-web_bg.wasm" "$OUT_DIR/agora-web_bg.wasm"
cp crates/agora-web/assets/loader.js "$OUT_DIR/"

echo "built $OUT_DIR:"
ls -l "$OUT_DIR"
