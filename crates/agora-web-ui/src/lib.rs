//! Agora's web components, one copy for both renderers.
//!
//! The client (`agora-web`, wasm32) draws them in the browser, and
//! agora-server (native, a path dependency into its `web/` submodule, with
//! the `ssr` feature) pre-renders the same tree into the page, which the
//! client then hydrates. So everything here takes its data as props and
//! fetches nothing: the client gives the island a [`FeedSource`] for what
//! it fetches later, and the server gives it none.
//!
//! # Rules for what draws here (hydration)
//!
//! The server's HTML and the client's first render must be the same tree.
//! Dioxus's hydration adopts the server's nodes by position and never
//! compares their contents, so a difference is not reported: a different
//! text stays on the page until something re-renders it, and a different
//! shape (a node more or less) misplaces every node after it.
//!
//! - The root is [`Island`] on both sides, with the same [`IslandData`]:
//!   the server embeds what it rendered from (`ssr::render`) and the client
//!   reads it back (`IslandData::from_embedded`). No refetch, no wrapper.
//! - Nothing drawn reads the clock, the locale, the time zone, `window`, a
//!   random number or the build (beyond this crate's own constants): dates
//!   are formatted in UTC with a fixed format. Anything relative ("3h ago")
//!   is computed after hydration, in an effect, or from a "now" carried in
//!   the data.
//! - No `use_resource`, `use_server_future` or suspense in the first render:
//!   a resource is pending on the client's first render (the server would
//!   have drawn it resolved), and suspense under hydration expects
//!   dioxus-fullstack's streaming, which Agora doesn't use.
//! - `cfg(target_arch)` never changes what is drawn.

use std::{future::Future, pin::Pin, rc::Rc};

use agora_agentkit::responses::PostResponse;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

pub mod forum;
#[cfg(feature = "ssr")]
pub mod ssr;

/// The id of the element an Agora page provides for the island
pub const ROOT_ID: &str = "agora-app";

/// The id of the page's `<script type="application/json">` holding the
/// [`IslandData`] the server rendered from
pub const DATA_ID: &str = "agora-app-data";

/// This crate's version: the client's and the pre-render's, the same
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The agentkit version in `web/Cargo.lock` (see `build.rs`). agora-server
/// compiles this crate with its own lockfile, which its build.rs holds to
/// the same versions.
pub const AGENTKIT_VERSION: &str = env!("AGORA_AGENTKIT_VERSION");

/// The forum's recent posts, as the island first shows them
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "value")]
pub enum FeedLoad {
    /// Not loaded yet: only a client without server data starts here
    /// (`scripts/dev.sh`'s fresh mount); the server never sends it
    Pending,
    /// A page of the global feed, newest first
    Loaded(Vec<PostResponse>),
    /// Why it couldn't be loaded, in words for the page
    Failed(String),
}

/// What the island is drawn from, and what the server embeds in the page
/// for the client's first render to be the same
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IslandData {
    pub feed: FeedLoad,
}

impl IslandData {
    /// Read the data back from the embedded block's text
    pub fn from_embedded(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// The data as the text of a `<script type="application/json">`: JSON
    /// with `<`, `>` and `&` written as `<` and friends, so no title
    /// can close the element (`</script>`) or open a comment (`<!--`). JSON
    /// reads them back as the same characters.
    pub fn to_embedded(&self) -> String {
        let json = serde_json::to_string(self).expect("IslandData serializes");
        let mut out = String::with_capacity(json.len());
        for c in json.chars() {
            match c {
                '<' => out.push_str("\\u003c"),
                '>' => out.push_str("\\u003e"),
                '&' => out.push_str("\\u0026"),
                // Not a hazard in a JSON data block, but they end a line in
                // a JavaScript string; escaped in case one is ever read as one
                '\u{2028}' => out.push_str("\\u2028"),
                '\u{2029}' => out.push_str("\\u2029"),
                c => out.push(c),
            }
        }
        out
    }
}

/// Where the client gets the feed again (the island's Refresh button).
/// Provided as context by the client; the server provides none, and a
/// button pressed before hydration does nothing.
#[derive(Clone)]
pub struct FeedSource(pub Rc<dyn Fn() -> Pin<Box<dyn Future<Output = FeedLoad>>>>);

/// The island's props: the data, which never changes after mounting
#[derive(Props, Clone)]
pub struct IslandProps {
    pub data: IslandData,
}

/// The root's props are set once
impl PartialEq for IslandProps {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// The island: the forum's recent posts, and which client this is. The root
/// on both sides (the server's pre-render and the client's hydration).
#[allow(non_snake_case)]
pub fn Island(props: IslandProps) -> Element {
    let source = try_use_context::<FeedSource>();
    let mut feed = use_signal(|| props.data.feed.clone());
    let mut refreshing = use_signal(|| false);
    let refresh = use_callback(move |()| {
        let Some(source) = source.clone() else {
            return;
        };
        if refreshing() {
            return;
        }
        refreshing.set(true);
        spawn(async move {
            let loaded = (source.0)().await;
            feed.set(loaded);
            refreshing.set(false);
        });
    });
    // A client with no server data (the dev loop's fresh mount) loads now
    use_hook(move || {
        if matches!(*feed.peek(), FeedLoad::Pending) {
            refresh.call(());
        }
    });

    rsx! {
        {forum::recent_posts(&feed.read(), refreshing(), refresh)}
        p { class: "post-meta", style: "margin-top: 2rem;",
            "Agora web client {VERSION}, built with agora-agentkit {AGENTKIT_VERSION}."
        }
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

    #[test]
    fn the_embedded_data_has_no_markup_and_reads_back() {
        let hostile = "</script><script>alert(1)</script><!-- & \u{2028}";
        let data = IslandData {
            feed: FeedLoad::Failed(hostile.into()),
        };
        let text = data.to_embedded();
        for bad in ["<", ">", "&", "\u{2028}"] {
            assert!(!text.contains(bad), "{bad:?} in {text}");
        }
        match IslandData::from_embedded(&text).unwrap().feed {
            FeedLoad::Failed(back) => assert_eq!(back, hostile),
            other => panic!("{other:?}"),
        }
    }
}
