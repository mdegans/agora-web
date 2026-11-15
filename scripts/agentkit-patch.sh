#!/bin/sh
# `just agentkit-patch` / `just agentkit-unpatch`: build against ./agentkit, a
# local agora-agentkit checkout or worktree, so that shared components
# hot-reload under `just dev` (README, "Editing shared components").
#   scripts/agentkit-patch.sh on    write .cargo/config.toml's [patch], point
#                                   Cargo.lock at it, and check that it took
#   scripts/agentkit-patch.sh off   undo `on`
# Never commit with it on: scripts/local-patch-check.sh (the hook, CI) refuses.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
CONFIG=.cargo/config.toml
MARK='# Written by `just agentkit-patch`; `just agentkit-unpatch` removes it.'
BACKUP=target/Cargo.lock.before-agentkit-patch

# Whether a lockfile (on stdin) has crates.io's agora-agentkit and no unused patch
lock_is_registry() {
    awk '
        $0 == "[[patch.unused]]" { unused = 1 }
        $0 == "name = \"agora-agentkit\"" { found = 1; next }
        found && $0 == "source = \"registry+https://github.com/rust-lang/crates.io-index\"" { ok = 1 }
        found && $0 == "" { found = 0 }
        END { exit ok && !unused ? 0 : 1 }
    '
}

# The workspace's agentkit requirement, e.g. 0.162.0
requirement() {
    sed -n 's/^agora-agentkit = { version = "\([^"]*\)".*/\1/p' Cargo.toml | head -1
}

ours() {
    [ -f "$CONFIG" ] && [ "$(head -1 "$CONFIG")" = "$MARK" ]
}

case "${1:-}" in
on)
    [ -f agentkit/Cargo.toml ] || {
        echo "error: no ./agentkit/Cargo.toml; put an agora-agentkit checkout or worktree there first (README, \"Editing shared components\")" >&2
        exit 1
    }
    [ ! -e "$CONFIG" ] || {
        echo "error: $CONFIG already exists; add the patch to it by hand, or move it aside" >&2
        exit 1
    }
    mkdir -p .cargo target
    cp Cargo.lock "$BACKUP"
    printf '%s\n%s\n%s\n' "$MARK" '[patch.crates-io]' 'agora-agentkit = { path = "agentkit" }' > "$CONFIG"
    checkout=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' agentkit/Cargo.toml | head -1)
    # Point the lockfile at it: until then cargo keeps the locked crates.io
    # version and lists the checkout under [[patch.unused]]
    if cargo update -p agora-agentkit 2>/dev/null &&
        cargo metadata --format-version 1 2>/dev/null |
        grep -q '"id":"path+file://[^"]*#agora-agentkit@' &&
        ! grep -q '^\[\[patch.unused\]\]' Cargo.lock; then
        echo "patched: agora-agentkit $checkout from ./agentkit; \`just agentkit-unpatch\` to undo"
        exit 0
    fi
    rm -f "$CONFIG"
    rmdir .cargo 2>/dev/null || true
    cp "$BACKUP" Cargo.lock
    echo "error: cargo didn't use ./agentkit ($checkout): this workspace requires agora-agentkit $(requirement), which $checkout doesn't satisfy. Check out a version it accepts, or bump the requirement in Cargo.toml locally (and don't commit that before agentkit is released). Nothing was changed." >&2
    exit 1
    ;;
off)
    had_config=
    if [ -f "$CONFIG" ]; then
        ours || {
            echo "error: $CONFIG wasn't written by agentkit-patch; edit it by hand" >&2
            exit 1
        }
        rm "$CONFIG"
        rmdir .cargo 2>/dev/null || true
        had_config=1
    fi
    # The lockfile only if the patch touched it (the working copy or the one
    # staged): any other change of yours stays
    if [ -n "$had_config" ] || ! lock_is_registry < Cargo.lock ||
        ! git show :Cargo.lock | lock_is_registry; then
        if ! git diff --quiet HEAD -- Cargo.lock || ! git diff --cached --quiet -- Cargo.lock; then
            mkdir -p target
            cp Cargo.lock target/Cargo.lock.before-unpatch
            # From HEAD, so a patched lockfile that was staged goes too
            git checkout HEAD -- Cargo.lock
            echo "restored Cargo.lock (the patched one is in target/Cargo.lock.before-unpatch)"
        fi
    fi
    scripts/local-patch-check.sh
    echo "unpatched: agora-agentkit from crates.io"
    ;;
*)
    echo "usage: scripts/agentkit-patch.sh on|off" >&2
    exit 2
    ;;
esac
