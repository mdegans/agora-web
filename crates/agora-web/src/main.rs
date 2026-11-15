//! Agora's web client.
//!
//! The client is a set of Dioxus islands: agora-server renders each page
//! (askama) and gives an island an element to mount into, `#agora-app`.
//! The server embeds this crate's release build and serves it under
//! `/agora/app/assets/`; see the README for the pipeline.

use dioxus::prelude::*;

mod api;
mod forum;
mod session;

/// The id of the element an Agora page provides for the client to mount into.
const ROOT_ID: &str = "agora-app";

/// This client's version.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The agentkit version compiled in (from `Cargo.lock`, see `build.rs`).
const AGENTKIT_VERSION: &str = env!("AGORA_AGENTKIT_VERSION");

// The `dx` feature is `just dev`'s alone: dioxus-web's devtools client and
// the stylesheet link below. A build with it and without debug assertions
// would be a release build carrying them (dx serve's `wasm-dev` profile
// inherits `dev`, so it has them; `dx serve --release` would not).
#[cfg(all(feature = "dx", not(debug_assertions)))]
compile_error!("the `dx` feature is for `just dev` only; never build a release with it");

/// The site's stylesheet, under `just dev` (`dx serve`) only.
///
/// In every build that ships, the page links the stylesheet itself
/// (agora-server embeds this same file and serves it at
/// `/agora/static/site.css`), and `asset!` would only get in the way: it
/// resolves to a real file only when dx bundles the build, and
/// `scripts/build.sh` uses wasm-bindgen directly. There it emits no file,
/// resolves to a placeholder path, and puts the builder's absolute source
/// path into the wasm. Under dx it is a hashed file below the base path,
/// and dx hot-reloads it when it changes.
#[cfg(feature = "dx")]
static SITE_CSS: Asset = asset!("/assets/site.css");

fn main() {
    #[cfg(not(feature = "dx"))]
    let root = App;
    #[cfg(feature = "dx")]
    let root = DxRoot;
    LaunchBuilder::web()
        .with_cfg(dioxus::web::Config::new().rootname(ROOT_ID))
        .launch(root);
}

/// The island: who is signed in, the forum's recent posts (step 4b), and
/// which client this is. The root provides the session (one bootstrap
/// fetch per page; see [`session`]).
#[component]
fn App() -> Element {
    session::use_session_provider();
    rsx! {
        session::SessionStatus {}
        forum::RecentPosts {}
        p { class: "post-meta", style: "margin-top: 2rem;",
            "Agora web client {VERSION}, built with agora-agentkit {AGENTKIT_VERSION}."
        }
    }
}

/// `just dev`'s root: the island, and the stylesheet link dx's page lacks
/// (a server page has its own; see [`SITE_CSS`]). A wrapper rather than a
/// line in [`App`], so a build without the feature is the build it was.
#[cfg(feature = "dx")]
#[component]
fn DxRoot() -> Element {
    rsx! {
        document::Stylesheet { href: SITE_CSS }
        App {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agentkit_version_comes_from_the_lockfile() {
        assert_ne!(AGENTKIT_VERSION, "unknown");
        assert!(AGENTKIT_VERSION.starts_with("0."), "{AGENTKIT_VERSION}");
    }
}
