# agora-web's recipes. `just` lists them. CI runs `just check`, and the
# pre-commit hook runs `just check-fast` (`just install-hooks` turns it on).

set shell := ["bash", "-euo", "pipefail", "-c"]

# List the recipes
default:
    @just --list

# The hot-reload loop: `dx serve` on the island, at http://localhost:PORT/agora/app/
# (8088 by default; Claude sessions on the shared box use 8089). Start agora's
# dev stack with `AGORA_DEV_PROXY_PORT=PORT just dev` for the API.
[doc("The hot-reload loop: dx serve on the island, at http://localhost:PORT/agora/app/")]
dev port=env("AGORA_WEB_DEV_PORT", "8088"):
    scripts/dx-serve.sh {{ quote(port) }}

# The real page under its real CSP: scripts/dev.sh (rebuild and reload, no
# hot patching). Arguments go to it, e.g. `just dev-page --upstream http://127.0.0.1:8181`
[doc("The real page under its real CSP (scripts/dev.sh): rebuild and reload, no hot patching")]
dev-page *args:
    scripts/dev.sh {{ args }}

# Everything CI checks: formatting, lints (host and wasm32), tests, the dev
# bundle and the release build
[doc("Everything CI checks: format, lints (host, wasm32), tests, the dev and release builds")]
check: check-fast test build-dev build

# What the pre-commit hook runs: formatting and lints, no tests or builds
check-fast: local-patch-check fmt-check clippy

# Format the workspace
fmt:
    cargo fmt --all

# Fail if anything isn't formatted
fmt-check:
    cargo fmt --all --check

# Clippy with warnings as errors: the host, wasm32, and wasm32 with `just dev`'s `dx` feature
clippy:
    cargo clippy --locked --workspace --all-targets -- -D warnings
    cargo clippy --locked -p agora-web --target wasm32-unknown-unknown -- -D warnings
    cargo clippy --locked -p agora-web --target wasm32-unknown-unknown --features dx -- -D warnings

# The tests, on the host (the client's included, with dioxus-ssr)
test:
    cargo test --locked --workspace

# The release build, into dist/ (the one agora embeds; scripts/build.sh)
build:
    scripts/build.sh dist

# The dev bundle scripts/dev.sh serves, into target/dev-dist
build-dev:
    scripts/build.sh --dev target/dev-dist

# Fail if a local agentkit patch is active (`just agentkit-patch`): what CI
# and agora build is crates.io's agentkit, never a checkout's
[doc("Fail if a local agentkit patch is active: CI and agora build crates.io's agentkit")]
local-patch-check:
    scripts/local-patch-check.sh

# Build against ./agentkit, a local agora-agentkit checkout or worktree, so
# edits to shared components hot-reload under `just dev` (README, "Editing
# shared components"). Never commit with it on: `just check` refuses.
[doc("Build against ./agentkit (a local agora-agentkit) so shared components hot-reload")]
agentkit-patch:
    #!/usr/bin/env bash
    set -euo pipefail
    [ -f agentkit/Cargo.toml ] || { echo "error: no ./agentkit/Cargo.toml; put an agora-agentkit checkout there first (README, \"Editing shared components\")" >&2; exit 1; }
    [ ! -e .cargo/config.toml ] || { echo "error: .cargo/config.toml already exists; add the patch to it by hand or move it aside" >&2; exit 1; }
    mkdir -p .cargo
    printf '%s\n' "# Written by \`just agentkit-patch\`; \`just agentkit-unpatch\` removes it." \
        '[patch.crates-io]' 'agora-agentkit = { path = "agentkit" }' > .cargo/config.toml
    # Point the lockfile at it: until then cargo keeps the locked crates.io
    # version and lists the checkout under [[patch.unused]]
    cargo update -p agora-agentkit
    echo "patched: agora-agentkit from ./agentkit ($(grep -m1 '^version' agentkit/Cargo.toml)); \`just agentkit-unpatch\` to undo"

# Undo `just agentkit-patch`: remove its config and restore Cargo.lock
agentkit-unpatch:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -f .cargo/config.toml ]; then
        head -1 .cargo/config.toml | grep -q '^# Written by `just agentkit-patch`' \
            || { echo "error: .cargo/config.toml wasn't written by agentkit-patch; edit it by hand" >&2; exit 1; }
        rm .cargo/config.toml
        rmdir .cargo 2>/dev/null || true
    fi
    # dx serve rewrote the lockfile for the patch. Kept aside first, in case
    # it held another change of yours.
    if ! git diff --quiet -- Cargo.lock; then
        mkdir -p target
        cp Cargo.lock target/Cargo.lock.before-unpatch
        git checkout -- Cargo.lock
        echo "restored Cargo.lock (the patched one is in target/Cargo.lock.before-unpatch)"
    fi
    scripts/local-patch-check.sh && echo "unpatched: agora-agentkit from crates.io"

# Install the build tools and `just dev`'s (dx, esbuild) into ~/.local/bin
install-tools:
    scripts/install-tools.sh --dev

# Use the versioned hooks in .githooks/ (pre-commit: `just check-fast`)
install-hooks:
    git config core.hooksPath .githooks
    @echo "hooks: $(git config core.hooksPath)"
