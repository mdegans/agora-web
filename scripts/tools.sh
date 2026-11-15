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

# --- Development only (`just dev`; scripts/install-tools.sh --dev) ---------
# Never used by build.sh, so agora's Docker stage and CI never fetch them.

# dioxus-cli (`dx`), the prebuilt release binary. It must equal the `dioxus`
# crate in Cargo.lock (dx refuses a mismatched devtools protocol), so a
# dioxus bump bumps this and all four hashes in the same change. The hashes
# are the ones the release publishes beside each tarball
# (https://github.com/DioxusLabs/dioxus/releases/download/v$DX_VERSION/dx-<platform>.sha256),
# which match the digests GitHub lists for the assets.
DX_VERSION=0.7.10

# Sets DX_PLATFORM and DX_SHA256 for this machine, or exits
dx_platform() {
    case "$(uname -s)-$(uname -m)" in
        Linux-x86_64)
            DX_PLATFORM=x86_64-unknown-linux-gnu
            DX_SHA256=4363e4ed2a3f1eb7f4d38d2d59aed59ce43271c44c16b425e92c89a64761fbe7 ;;
        Linux-aarch64 | Linux-arm64)
            DX_PLATFORM=aarch64-unknown-linux-gnu
            DX_SHA256=8f1a17d3218700ffbe15e6540d936a178b2556fc801121a31082e3ba4ab9ef55 ;;
        Darwin-arm64)
            DX_PLATFORM=aarch64-apple-darwin
            DX_SHA256=342c92bc33dca60dadd725ed5e249923f049ae2e1e50b7477f4968d56695b425 ;;
        Darwin-x86_64)
            DX_PLATFORM=x86_64-apple-darwin
            DX_SHA256=e3edbba285fa23a834aa7ceb558c9d8088cb3fbdcb48fb8c619532ed4d91cab7 ;;
        *)
            echo "error: no dx release for $(uname -s) $(uname -m); install dioxus-cli $DX_VERSION yourself" >&2
            exit 1 ;;
    esac
}

# esbuild, which dx insists on finding before it builds for the web (it
# minifies JS assets with it; this client has none). Left to itself dx
# downloads esbuild, and wasm-bindgen-cli, into its own data directory with
# no checksum; `just dev` runs it with NO_DOWNLOADS=1 instead, so it takes
# both from PATH: wasm-bindgen at Cargo.lock's version, and this esbuild.
# The version is the one dx 0.7.10 itself fetches. esbuild publishes its
# binaries on npm only, so the hashes are npm's `dist.integrity` (SHA-512,
# base64 there, hex here) for each @esbuild/<platform> package
# (https://registry.npmjs.org/@esbuild/<platform>/$ESBUILD_VERSION).
ESBUILD_VERSION=0.27.3

# Sets ESBUILD_PLATFORM and ESBUILD_SHA512 for this machine, or exits
esbuild_platform() {
    case "$(uname -s)-$(uname -m)" in
        Linux-x86_64)
            ESBUILD_PLATFORM=linux-x64
            ESBUILD_SHA512=0b38bccb35d458841802d2ffdb2eafa20111f29bdcb0eb24e5ca702f81a4e6726a1f9519895072218f04a2b9b9475de0abe0d0298834f89c90a32b4b41ab3874 ;;
        Linux-aarch64 | Linux-arm64)
            ESBUILD_PLATFORM=linux-arm64
            ESBUILD_SHA512=b193ae173ff15a7678287dd87c5aca09fd56c8f6476a45734e2aa38b7583734042976901c2224b097a732f35012e09a9e2f78565dbcde42856e04abff0573616 ;;
        Darwin-arm64)
            ESBUILD_PLATFORM=darwin-arm64
            ESBUILD_SHA512=45ee3dd64ec1c93551cb4b7710a59a8dd2c8af4833da428a7f369f92d87843c0399f5c531eb92a6602cb8c5107543f805dd50681032af86a1d7eae39986a422a ;;
        Darwin-x86_64)
            ESBUILD_PLATFORM=darwin-x64
            ESBUILD_SHA512=bc793f840effd407248c6cd1aa2eb06e8fa3692873468c188a9eabb7aabb5581035f82c4cb5a597c3a5dc420671ad97e039ceaf225c3732bb1c2dbf784db4716 ;;
        *)
            echo "error: no esbuild release for $(uname -s) $(uname -m); install esbuild $ESBUILD_VERSION yourself" >&2
            exit 1 ;;
    esac
}

# Exits unless DX_VERSION (above) is Cargo.lock's dioxus
require_dx_pin() {
    DIOXUS_VERSION=$(lock_version dioxus)
    if [ "$DX_VERSION" != "$DIOXUS_VERSION" ]; then
        echo "error: Cargo.lock has dioxus $DIOXUS_VERSION but scripts/tools.sh pins dx $DX_VERSION; bump DX_VERSION and its hashes together" >&2
        exit 1
    fi
}

# Exits unless dx is on PATH at Cargo.lock's dioxus version, and esbuild at
# its pin
require_dx() {
    require_dx_pin
    if [ "$(dx --version 2>/dev/null | cut -d' ' -f1-2)" != "dioxus $DX_VERSION" ]; then
        echo "error: need dx $DX_VERSION (dioxus in Cargo.lock), found: $(dx --version 2>/dev/null || echo none); run scripts/install-tools.sh --dev" >&2
        exit 1
    fi
    if [ "$(esbuild --version 2>/dev/null)" != "$ESBUILD_VERSION" ]; then
        echo "error: need esbuild $ESBUILD_VERSION on PATH for dx; run scripts/install-tools.sh --dev" >&2
        exit 1
    fi
}

# Downloads URL to FILE with curl or wget
fetch() {
    if command -v curl >/dev/null; then
        curl -fsSL -o "$2" "$1"
    else
        wget -q -O "$2" "$1"
    fi
}

# Checks FILE against a hex digest: `check_sum 256|512 DIGEST FILE`
# (macOS has shasum, not sha256sum)
check_sum() {
    if command -v "sha$1sum" >/dev/null; then
        echo "$2  $3" | "sha$1sum" -c -
    else
        echo "$2  $3" | shasum -a "$1" -c -
    fi
}
