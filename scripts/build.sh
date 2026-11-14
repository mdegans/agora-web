#!/bin/sh
# Builds the release client into OUT_DIR (default dist/): the wasm32 build,
# wasm-bindgen (`--target web`), wasm-opt, and the loader. The one recipe
# for every build: this repo's CI, agora's Docker stage and `just web-build`.
# `--dev` builds the dev bundle scripts/dev.sh serves instead: Cargo's dev
# profile (unoptimized, so a rebuild takes seconds) and no wasm-opt.
# Usage: scripts/build.sh [--dev] [OUT_DIR]
# Install the tools first with scripts/install-tools.sh.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
. "$ROOT/scripts/tools.sh"
DEV=
if [ "${1:-}" = "--dev" ]; then
    DEV=1
    shift
fi
OUT_DIR=${1:-$ROOT/dist}
TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target}

require_wasm_bindgen
if [ -z "$DEV" ] && ! wasm-opt --version 2>/dev/null | grep -q "^wasm-opt version $BINARYEN_VERSION "; then
    echo "error: need wasm-opt $BINARYEN_VERSION; run scripts/install-tools.sh" >&2
    exit 1
fi

cd "$ROOT"
if [ -n "$DEV" ]; then
    cargo build --locked --target wasm32-unknown-unknown -p agora-web
    PROFILE_DIR=debug
else
    cargo build --release --locked --target wasm32-unknown-unknown -p agora-web
    PROFILE_DIR=release
fi

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"
wasm-bindgen --target web --no-typescript --out-name agora-web --out-dir "$OUT_DIR" \
    "$TARGET_DIR/wasm32-unknown-unknown/$PROFILE_DIR/agora-web.wasm"
cp crates/agora-web/assets/loader.js "$OUT_DIR/"
if [ -n "$DEV" ]; then
    echo "built the dev bundle in $OUT_DIR"
    exit 0
fi
# The features rustc enables by default for wasm32-unknown-unknown (Rust
# 1.82+); wasm-opt refuses input that uses features it wasn't given. Every
# current browser supports them.
wasm-opt -Oz --strip-debug --strip-producers \
    --enable-bulk-memory --enable-bulk-memory-opt --enable-multivalue \
    --enable-mutable-globals --enable-nontrapping-float-to-int \
    --enable-reference-types --enable-sign-ext \
    -o "$OUT_DIR/agora-web_bg.wasm" "$OUT_DIR/agora-web_bg.wasm"

echo "built $OUT_DIR:"
ls -l "$OUT_DIR"
