#!/bin/sh
# `just dev`: the hot-reload loop, `dx serve` on the island. An edit inside an
# `rsx!` (text, attributes, elements) patches the open page in place; any
# other change rebuilds (a few seconds) and reloads it. The stylesheet
# (crates/agora-web/assets/site.css) hot-reloads too. API calls under
# /agora/api/ go to the agora dev stack (Dioxus.toml): start it in agora with
# `AGORA_DEV_PROXY_PORT=<this port> just dev`.
#
# Usage: scripts/dx-serve.sh [PORT]   (or AGORA_WEB_DEV_PORT; default 8088)
#   then http://localhost:PORT/agora/app/
# Claude sessions on the shared box use 8089, so they don't take the
# Steward's 8088.
#
# Needs dx and esbuild at the pins in scripts/tools.sh
# (scripts/install-tools.sh --dev) and wasm-bindgen at Cargo.lock's version.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
. "$ROOT/scripts/tools.sh"
PORT=${1:-${AGORA_WEB_DEV_PORT:-8088}}
case "$PORT" in
    '' | *[!0-9]*) echo "error: port must be a number, got '$PORT'" >&2; exit 1 ;;
esac
require_dx
require_wasm_bindgen
# With `just agentkit-patch` on, a checkout whose version the workspace's
# requirement no longer accepts (after a pull, say) is silently replaced by
# crates.io's: say so before serving, not after an hour of edits that do
# nothing
"$ROOT/scripts/agentkit-patch.sh" check

# No downloads: dx takes wasm-bindgen and esbuild from PATH, at the versions
# just checked, instead of fetching its own with no checksum. Its data
# directory (settings, logs) is under target/, not the home directory, so no
# tool it may have fetched before is used. No telemetry.
export NO_DOWNLOADS=1
export DX_HOME="${CARGO_TARGET_DIR:-$ROOT/target}/dx-home"
export DX_TELEMETRY_ENABLED=false
export TELEMETRY=false

cd "$ROOT/crates/agora-web"
# --features dx: the devtools client and the island's own stylesheet link
# (Cargo.toml). The profile is wasm-dev (no debug info; the workspace
# Cargo.toml). --hot-patch (Rust code patching) doesn't build on this
# toolchain, so non-rsx changes rebuild.
# dx's key commands (r to rebuild, v for verbose logging, / for more) need
# its interactive mode, which needs a terminal. In one, it is on; with
# output piped or no terminal (a script, an agent's log), dx prints plain
# lines instead
if [ -t 0 ] && [ -t 1 ]; then INTERACTIVE=true; else INTERACTIVE=false; fi
exec dx serve --web --features dx --profile wasm-dev \
    --addr 127.0.0.1 --port "$PORT" \
    --open false --interactive "$INTERACTIVE"
