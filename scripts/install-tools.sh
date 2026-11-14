#!/bin/sh
# Installs the tools scripts/build.sh needs, at the versions it checks:
#   wasm-bindgen-cli (cargo install, at Cargo.lock's wasm-bindgen version)
#   wasm-opt (binaryen's release binary, SHA-256 checked) into BIN_DIR
# Usage: scripts/install-tools.sh [BIN_DIR]   (default ~/.local/bin)
# Skips a tool that is already on PATH at the right version. Needs cargo,
# curl or wget, sha256sum and tar; the wasm32 target is added with rustup.
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
    tarball="binaryen-version_${BINARYEN_VERSION}-x86_64-linux.tar.gz"
    url="https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/$tarball"
    tmp=$(mktemp -d)
    if command -v curl >/dev/null; then
        curl -fsSL -o "$tmp/$tarball" "$url"
    else
        wget -q -O "$tmp/$tarball" "$url"
    fi
    echo "$BINARYEN_SHA256  $tmp/$tarball" | sha256sum -c -
    tar -xzf "$tmp/$tarball" -C "$tmp"
    cp "$tmp/binaryen-version_${BINARYEN_VERSION}/bin/wasm-opt" "$BIN_DIR/wasm-opt"
    rm -rf "$tmp"
fi

if command -v rustup >/dev/null; then
    rustup target add wasm32-unknown-unknown
fi
