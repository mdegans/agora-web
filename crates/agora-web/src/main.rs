//! Agora's web client.
//!
//! The client is a set of Dioxus islands. agora-server renders each page
//! (askama) and pre-renders the island inside its element, `#agora-app`,
//! from agentkit's shared components (`agora_agentkit::ui`, the one copy of
//! what an island draws), with the data it drew them from in a JSON block.
//! This binary reads that block and adopts the server's nodes (hydration):
//! no refetch, no inline script. The server embeds this crate's release
//! build and serves it under `/agora/app/assets/`; see the README ("Server
//! rendering and hydration", and the pipeline).
//!
//! Two builds ([`mount`]): the release build (`hydrate` feature,
//! `scripts/build.sh`) hydrates, and stays off a page it can't hydrate; the
//! dev build (`scripts/build.sh --dev`) clears the island's element and
//! mounts fresh, loading the feed itself.
//!
//! This crate keeps what is the client's alone: fetching ([`api`]), who is
//! signed in ([`session`]) and mounting ([`mount`]). What is drawn is
//! agentkit's.

use agora_agentkit::ui::{
    Reload,
    feed::{FeedIsland, FeedLoad, FeedPage},
    island::ROOT_ID,
};
use dioxus::prelude::*;

mod api;
mod mount;
mod session;

// The `dx` feature is `just dev`'s alone: dioxus-web's devtools client and
// the stylesheet link below. A build with it and without debug assertions
// would be a release build carrying them (dx serve's `wasm-dev` profile
// inherits `dev`, so it has them; `dx serve --release` would not).
#[cfg(all(feature = "dx", not(debug_assertions)))]
compile_error!("the `dx` feature is for `just dev` only; never build a release with it");

// dx serves its own page, with no server markup to adopt: it mounts fresh
#[cfg(all(feature = "dx", feature = "hydrate"))]
compile_error!("the `dx` feature mounts fresh; it can't be combined with `hydrate`");

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
    // What `LaunchBuilder::launch` would do first; the island is launched
    // with its props (`launch`), which the builder can't pass
    dioxus::logger::initialize_default();
    #[cfg(feature = "hydrate")]
    mount::hydrate();
    #[cfg(not(feature = "hydrate"))]
    mount::fresh();
}

/// The feed island's [`Reload`]: the page of the feed the server drew,
/// fetched again (the Refresh and Try again buttons, and a fresh mount's
/// first load)
fn reload_feed() -> Reload<FeedLoad> {
    Reload::new(|| {
        Box::pin(async {
            match api::feed(&FeedPage::input()).await {
                Ok(posts) => posts.into_iter().collect(),
                Err(e) => FeedLoad::Failed(e.to_string()),
            }
        })
    })
}

/// Launches the island: agentkit's [`FeedIsland`], the root the server
/// pre-rendered, with `page` as its props and [`reload_feed`] in context.
/// Under dx, [`DxRoot`] around it (dx mounts fresh, never hydrates)
fn launch(page: FeedPage) {
    #[cfg(not(feature = "dx"))]
    let dom = VirtualDom::new_with_props(FeedIsland, page);
    #[cfg(feature = "dx")]
    let dom = VirtualDom::new_with_props(DxRoot, DxRootProps { page });
    let dom = dom.with_root_context(reload_feed());
    dioxus::web::launch::launch_virtual_dom(dom, dioxus::web::Config::new().rootname(ROOT_ID));
}

/// `just dev`'s root: the island, and the stylesheet link dx's page lacks
/// (a server page has its own; see [`SITE_CSS`]). A wrapper rather than a
/// line in the island, so a build without the feature is the build it was:
/// the shared root, unwrapped, which is what hydration needs
#[cfg(feature = "dx")]
#[component]
fn DxRoot(page: FeedPage) -> Element {
    rsx! {
        document::Stylesheet { href: SITE_CSS }
        FeedIsland { feed: page.feed }
    }
}
