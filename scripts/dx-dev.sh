#!/bin/sh
# SPIKE (spike/dx-serve): the hot-reload loop, `dx serve`, as `just dev`
# would run it. rsx edits patch the open page without a reload; any other
# change rebuilds (about 2 s) and reloads it. API calls under /agora/api/
# go to the agora dev stack's proxy listener (Dioxus.toml); start it with
# `AGORA_DEV_PROXY_PORT=<this port> just dev` in agora so its public URL
# is this page's origin.
# Needs dx at the dioxus version in Cargo.lock (dioxus-cli 0.7.10).
# Usage: scripts/dx-dev.sh [PORT]   (default 8089; then
#   http://localhost:PORT/agora/app/)
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
. "$ROOT/scripts/tools.sh"
DIOXUS_VERSION=$(lock_version dioxus)
if [ "$(dx --version 2>/dev/null | cut -d' ' -f2)" != "$DIOXUS_VERSION" ]; then
    echo "error: need dx $DIOXUS_VERSION (dioxus in Cargo.lock)" >&2
    exit 1
fi
cd "$ROOT/crates/agora-web"
exec dx serve --web --features dx --addr 127.0.0.1 --port "${1:-8089}" \
    --open false --interactive false
