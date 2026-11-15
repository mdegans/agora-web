#!/bin/sh
# `just agentkit-patch` / `just agentkit-unpatch`: build against ./agentkit, a
# local agora-agentkit checkout or worktree, so that shared components
# hot-reload under `just dev` (README, "Editing shared components").
#   scripts/agentkit-patch.sh on    write .cargo/config.toml's [patch], point
#                                   Cargo.lock at it, and check that it took
#   scripts/agentkit-patch.sh off   undo `on`
#   scripts/agentkit-patch.sh check `just dev` runs it: with the patch on, fail
#                                   unless cargo really builds ./agentkit
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

# The checkout's version, e.g. 0.164.0
checkout_version() {
    sed -n 's/^version = "\([^"]*\)".*/\1/p' agentkit/Cargo.toml | head -1
}

# Whether cargo resolves agora-agentkit to ./agentkit, with no unused patch
in_use() {
    cargo metadata --format-version 1 2>/dev/null |
        grep -q '"id":"path+file://[^"]*#agora-agentkit@' &&
        ! grep -q '^\[\[patch.unused\]\]' Cargo.lock
}

# The agentkit this workspace is locked to (the committed lockfile), e.g.
# 0.162.1: the version the client is built and tested against
locked() {
    git show HEAD:Cargo.lock 2>/dev/null |
        awk '$0 == "name = \"agora-agentkit\"" { f = 1; next } f && /^version = / { gsub(/[^0-9.]/, "", $3); print $3; exit }'
}

# Why cargo won't use the checkout, and the two ways out. A `[patch]` only
# applies when the patched crate's version satisfies the requirement;
# otherwise cargo takes crates.io's without a word louder than a warning
mismatch() {
    echo "error: cargo isn't building ./agentkit ($(checkout_version)): this workspace requires agora-agentkit $(requirement), which $(checkout_version) doesn't satisfy, so cargo uses crates.io's and edits in ./agentkit do nothing." >&2
    echo "  Either put the checkout on a version it accepts:  git -C agentkit checkout v$(locked)   (the version this workspace is locked to; or a branch made from it)" >&2
    echo "  or, to work on a newer agentkit, raise the requirement here to match it: edit agora-agentkit's version in Cargo.toml to \"$(checkout_version)\" (agora's build then wants the same in agora and agents; commit it only once that agentkit is released)." >&2
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
    checkout=$(checkout_version)
    # Point the lockfile at it: until then cargo keeps the locked crates.io
    # version and lists the checkout under [[patch.unused]]
    if cargo update -p agora-agentkit 2>/dev/null && in_use; then
        echo "patched: agora-agentkit $checkout from ./agentkit; \`just agentkit-unpatch\` to undo"
        exit 0
    fi
    rm -f "$CONFIG"
    rmdir .cargo 2>/dev/null || true
    cp "$BACKUP" Cargo.lock
    mismatch
    echo "  Nothing was changed." >&2
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
check)
    # Not patched: nothing to check
    ours || exit 0
    [ -f agentkit/Cargo.toml ] || {
        echo "error: $CONFIG patches agora-agentkit to ./agentkit, which is gone; \`just agentkit-unpatch\`, or put the checkout back" >&2
        exit 1
    }
    # The checkout may have moved since `on` (a pull, a version bump): point
    # the lockfile at it again, then see whether cargo takes it
    cargo update -p agora-agentkit 2>/dev/null || true
    in_use && exit 0
    mismatch
    echo "  (\`just agentkit-unpatch\` to go back to crates.io's.)" >&2
    exit 1
    ;;
*)
    echo "usage: scripts/agentkit-patch.sh on|off|check" >&2
    exit 2
    ;;
esac
