#!/bin/sh
# Installs the tools scripts/build.sh needs, at the versions it checks:
#   wasm-bindgen-cli (cargo install, at Cargo.lock's wasm-bindgen version)
#   wasm-opt (binaryen's release binary, SHA-256 checked) into BIN_DIR
# With --dev, also what `just dev` needs (scripts/tools.sh has the pins):
#   dx (dioxus-cli's release binary, SHA-256 checked) into BIN_DIR
#   esbuild (npm's release binary, SHA-512 checked) into BIN_DIR
# Usage: scripts/install-tools.sh [--dev] [BIN_DIR]   (default ~/.local/bin)
# Without --dev it installs exactly what it always has: agora's Docker stage
# and `just web-build` run it that way, and never fetch the dev tools.
# Skips a tool that is already on PATH at the right version. Needs cargo,
# curl or wget, sha256sum/sha512sum or shasum, and tar; the wasm32 target is
# added with rustup.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
. "$ROOT/scripts/tools.sh"
DEV=
if [ "${1:-}" = "--dev" ]; then
    DEV=1
    shift
fi
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
    fetch "$url" "$tmp/$tarball"
    check_sum 256 "$BINARYEN_SHA256" "$tmp/$tarball"
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

if [ -n "$DEV" ]; then
    require_dx_pin
    if [ "$(dx --version 2>/dev/null | cut -d' ' -f1-2)" != "dioxus $DX_VERSION" ]; then
        dx_platform
        tarball="dx-$DX_PLATFORM.tar.gz"
        tmp=$(mktemp -d)
        fetch "https://github.com/DioxusLabs/dioxus/releases/download/v$DX_VERSION/$tarball" "$tmp/$tarball"
        check_sum 256 "$DX_SHA256" "$tmp/$tarball"
        tar -xzf "$tmp/$tarball" -C "$tmp" dx
        cp "$tmp/dx" "$BIN_DIR/dx"
        chmod 755 "$BIN_DIR/dx"
        rm -rf "$tmp"
        "$BIN_DIR/dx" --version
    fi
    if [ "$(esbuild --version 2>/dev/null)" != "$ESBUILD_VERSION" ]; then
        esbuild_platform
        tarball="$ESBUILD_PLATFORM-$ESBUILD_VERSION.tgz"
        tmp=$(mktemp -d)
        fetch "https://registry.npmjs.org/@esbuild/$ESBUILD_PLATFORM/-/$tarball" "$tmp/$tarball"
        check_sum 512 "$ESBUILD_SHA512" "$tmp/$tarball"
        tar -xzf "$tmp/$tarball" -C "$tmp" package/bin/esbuild
        cp "$tmp/package/bin/esbuild" "$BIN_DIR/esbuild"
        chmod 755 "$BIN_DIR/esbuild"
        rm -rf "$tmp"
        echo "esbuild $("$BIN_DIR/esbuild" --version)"
    fi
fi

if command -v rustup >/dev/null; then
    rustup target add wasm32-unknown-unknown
fi
