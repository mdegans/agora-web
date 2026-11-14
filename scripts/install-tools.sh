#!/bin/sh
# Installs the tools scripts/build.sh needs, at the versions it checks:
#   wasm-bindgen-cli (cargo install, at Cargo.lock's wasm-bindgen version)
#   wasm-opt (binaryen's release binary, SHA-256 checked) into BIN_DIR
# Usage: scripts/install-tools.sh [BIN_DIR]   (default ~/.local/bin)
# Skips a tool that is already on PATH at the right version. Needs cargo,
# curl or wget, sha256sum or shasum, and tar; the wasm32 target is added with rustup.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
. "$ROOT/scripts/tools.sh"
BIN_DIR=${1:-$HOME/.local/bin}
mkdir -p "$BIN_DIR"

WASM_BINDGEN_VERSION=$(lock_version wasm-bindgen)
if [ "$(wasm-bindgen --version 2>/dev/null)" != "wasm-bindgen $WASM_BINDGEN_VERSION" ]; then
    cargo install wasm-bindgen-cli --version "=$WASM_BINDGEN_VERSION" --locked
fi

if ! wasm-opt --version 2>/dev/null | grep -q "^wasm-opt version $BINARYEN_VERSION "; then
    binaryen_platform
    tarball="binaryen-version_${BINARYEN_VERSION}-${BINARYEN_PLATFORM}.tar.gz"
    url="https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/$tarball"
    tmp=$(mktemp -d)
    if command -v curl >/dev/null; then
        curl -fsSL -o "$tmp/$tarball" "$url"
    else
        wget -q -O "$tmp/$tarball" "$url"
    fi
    # macOS has shasum, not sha256sum
    if command -v sha256sum >/dev/null; then
        echo "$BINARYEN_SHA256  $tmp/$tarball" | sha256sum -c -
    else
        echo "$BINARYEN_SHA256  $tmp/$tarball" | shasum -a 256 -c -
    fi
    tar -xzf "$tmp/$tarball" -C "$tmp"
    cp "$tmp/binaryen-version_${BINARYEN_VERSION}/bin/wasm-opt" "$BIN_DIR/wasm-opt"
    # The Linux binary is static. The macOS one loads libbinaryen.dylib from
    # ../lib beside itself (its rpath is @loader_path/../lib).
    dylib="$tmp/binaryen-version_${BINARYEN_VERSION}/lib/libbinaryen.dylib"
    if [ -f "$dylib" ]; then
        mkdir -p "$BIN_DIR/../lib"
        cp "$dylib" "$BIN_DIR/../lib/libbinaryen.dylib"
    fi
    rm -rf "$tmp"
    "$BIN_DIR/wasm-opt" --version
fi

if command -v rustup >/dev/null; then
    rustup target add wasm32-unknown-unknown
fi
