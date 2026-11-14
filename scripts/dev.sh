#!/bin/sh
# The client's development loop: builds the dev bundle (scripts/build.sh
# --dev), rebuilds it when crates/agora-web changes, and serves it on
# 127.0.0.1 under the upstream's own /agora/app page and CSP, proxying the
# API and everything else to the upstream (crates/dev-server).
# Usage: scripts/dev.sh [--port PORT] [--upstream URL]
#   (or AGORA_WEB_DEV_PORT / AGORA_WEB_DEV_UPSTREAM; defaults 8088 and
#   https://subliminal.technology). Then open http://127.0.0.1:8088/agora/app
# Needs the wasm32 target and wasm-bindgen: scripts/install-tools.sh.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
. "$ROOT/scripts/tools.sh"

WASM_BINDGEN_VERSION=$(lock_version wasm-bindgen)
if [ "$(wasm-bindgen --version 2>/dev/null)" != "wasm-bindgen $WASM_BINDGEN_VERSION" ]; then
    echo "error: need wasm-bindgen $WASM_BINDGEN_VERSION (Cargo.lock); run scripts/install-tools.sh" >&2
    exit 1
fi

cd "$ROOT"
exec cargo run --locked --quiet -p agora-web-dev -- "$@"
