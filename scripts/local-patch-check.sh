#!/bin/sh
# Fails if agora-agentkit wouldn't come from crates.io. A local agentkit
# (`just agentkit-patch`; README, "Editing shared components") is for
# `just dev` only: what CI checks and agora builds is crates.io's agentkit at
# Cargo.lock's version, so a patched lockfile must never be committed, and
# a check run against a patch must never pass. Run by `just check-fast` (so
# by the pre-commit hook) and `just check` (so by CI).
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
HOW="a local agentkit is for \`just dev\` only: \`just agentkit-unpatch\` (or move .cargo/config.toml aside and \`git checkout Cargo.lock\`)"

# The lockfile: agentkit's entry must name crates.io. dx serve, which doesn't
# pass --locked, rewrites it under a patch.
if ! awk '
    $0 == "name = \"agora-agentkit\"" { found = 1; next }
    found && $0 == "source = \"registry+https://github.com/rust-lang/crates.io-index\"" { ok = 1; exit }
    found && $0 == "" { exit }
    END { exit ok ? 0 : 1 }
' Cargo.lock; then
    echo "error: Cargo.lock's agora-agentkit isn't crates.io's; $HOW" >&2
    exit 1
fi

# What cargo resolves now, with every config file it reads (this checkout's
# .cargo/, its parents', CARGO_HOME's). --locked: a patch the lockfile
# doesn't record fails here rather than rewriting it.
if ! metadata=$(cargo metadata --locked --format-version 1 2>&1); then
    echo "$metadata" >&2
    echo "error: cargo can't resolve the workspace against Cargo.lock; if a [patch] is active, $HOW" >&2
    exit 1
fi
if echo "$metadata" | grep -q '"id":"[^"]*#agora-agentkit@' &&
    ! echo "$metadata" | grep -q '"id":"registry+https://github.com/rust-lang/crates.io-index#agora-agentkit@'; then
    echo "error: agora-agentkit resolves to something other than crates.io; $HOW" >&2
    exit 1
fi
