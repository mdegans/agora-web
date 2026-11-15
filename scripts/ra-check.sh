#!/bin/sh
# rust-analyzer's check on save (.vscode/settings.json): clippy for the host,
# the whole workspace with its tests, then for wasm32, the client alone (the
# dev server is native only) with `just dev`'s `dx` feature, as one stream
# of JSON diagnostics. Between them both sides of every `cfg(feature = "dx")`
# are checked (the host run has it off) without a third run. So an error
# that only the browser build has shows in the editor too. A target directory
# of its own, so it never waits on `just dev`'s build for the lock.
# Lints as `just clippy` has them, but warnings stay warnings here.
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}/rust-analyzer"
cargo clippy --workspace --all-targets --message-format=json-diagnostic-rendered-ansi \
    --target-dir "$TARGET_DIR"
cargo clippy -p agora-web --target wasm32-unknown-unknown --features dx --message-format=json-diagnostic-rendered-ansi \
    --target-dir "$TARGET_DIR"
