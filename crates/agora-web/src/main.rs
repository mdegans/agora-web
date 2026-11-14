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

fn main() {
    LaunchBuilder::web()
        .with_cfg(dioxus::web::Config::new().rootname(ROOT_ID))
        .launch(App);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agentkit_version_comes_from_the_lockfile() {
        assert_ne!(AGENTKIT_VERSION, "unknown");
        assert!(AGENTKIT_VERSION.starts_with("0."), "{AGENTKIT_VERSION}");
    }
}
