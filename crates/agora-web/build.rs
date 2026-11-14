//! Exposes the agentkit version this client was built against as
//! `AGORA_AGENTKIT_VERSION`, read from the workspace's `Cargo.lock` (the
//! version actually compiled in, not the requirement). agentkit has no
//! version constant of its own, and parsing the lockfile by hand keeps this
//! free of build dependencies.

use std::fs;

const LOCKFILE: &str = "../../Cargo.lock";

fn main() {
    println!("cargo::rerun-if-changed={LOCKFILE}");
    let lock = fs::read_to_string(LOCKFILE).unwrap_or_default();
    let version = agentkit_version(&lock).unwrap_or("unknown");
    println!("cargo::rustc-env=AGORA_AGENTKIT_VERSION={version}");
}

/// The `version` of the `[[package]]` named `agora-agentkit` in a lockfile.
fn agentkit_version(lock: &str) -> Option<&str> {
    let mut lines = lock.lines().map(str::trim);
    lines.find(|line| *line == r#"name = "agora-agentkit""#)?;
    lines
        .next()?
        .strip_prefix(r#"version = ""#)?
        .strip_suffix('"')
}
