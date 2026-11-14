# The build's tool versions, in one place. Sourced by install-tools.sh and
# build.sh (POSIX sh), so agora's Docker stage, its `just web-build` and this
# repo's CI all build with the same tools.

# wasm-bindgen-cli must equal the `wasm-bindgen` crate in Cargo.lock exactly
# (the CLI refuses a mismatched schema), so it is read from the lockfile
# rather than written down a second time.
lock_version() {
    awk -v name="$1" '
        $0 == "name = \"" name "\"" { found = 1; next }
        found { gsub(/^version = "|"$/, ""); print; exit }
    ' "$ROOT/Cargo.lock"
}

# binaryen's wasm-opt, a release binary, checked against its SHA-256. One
# tarball per platform (the names are binaryen's); bump the version and all
# four hashes together (each release publishes a `.sha256` beside its tarball).
BINARYEN_VERSION=133

# Sets BINARYEN_PLATFORM and BINARYEN_SHA256 for this machine, or exits
binaryen_platform() {
    case "$(uname -s)-$(uname -m)" in
        Linux-x86_64)
            BINARYEN_PLATFORM=x86_64-linux
            BINARYEN_SHA256=2dc9c7813f5375db93d96ead4b78222fcc3e2677bbb832297af4797782a37489 ;;
        Linux-aarch64 | Linux-arm64)
            BINARYEN_PLATFORM=aarch64-linux
            BINARYEN_SHA256=89c07ea56faf38d0fbecf36ca8ec0721756716185f265b568e133d427f299bf8 ;;
        Darwin-arm64)
            BINARYEN_PLATFORM=arm64-macos
            BINARYEN_SHA256=ad66da82ac13f163e424b1643f16c6dfcccc98b5966296b43e52d3cab04f84a8 ;;
        Darwin-x86_64)
            BINARYEN_PLATFORM=x86_64-macos
            BINARYEN_SHA256=13a9b90be775c6389ce3d1f879cb8627bea56708ba8c122983941d53a8199b95 ;;
        *)
            echo "error: no binaryen release for $(uname -s) $(uname -m); install wasm-opt $BINARYEN_VERSION yourself" >&2
            exit 1 ;;
    esac
}

# Exits unless wasm-bindgen is on PATH at Cargo.lock's version
require_wasm_bindgen() {
    WASM_BINDGEN_VERSION=$(lock_version wasm-bindgen)
    if [ "$(wasm-bindgen --version 2>/dev/null)" != "wasm-bindgen $WASM_BINDGEN_VERSION" ]; then
        echo "error: need wasm-bindgen $WASM_BINDGEN_VERSION (Cargo.lock); run scripts/install-tools.sh" >&2
        exit 1
    fi
}
