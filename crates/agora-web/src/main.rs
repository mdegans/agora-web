//! Agora's web client.
//!
//! agora-server renders each forum page (the feeds, a post's thread, a
//! comment's page) through agentkit's router (`ui::route`), pre-rendering
//! the site's root inside its element, `#agora-app`, from agentkit's shared
//! components (`agora_agentkit::ui`, the one copy of what a page draws),
//! with the page it drew them from in a JSON block. This binary reads that
//! block and adopts the server's nodes (hydration): no refetch, no inline
//! script. From there a link to another of those pages navigates in the
//! page ([`site`]): the client loads that page from the public API and
//! draws it, and the browser's address, history and title follow. Every
//! other link is a full page load. The server embeds this crate's release
//! build and serves it under `/agora/app/assets/`; see the README ("Server
//! rendering and hydration", and the pipeline).
//!
//! Two builds ([`mount`]): the release build (`hydrate` feature,
//! `scripts/build.sh`) hydrates, and stays off a page it can't hydrate; the
//! dev build (`scripts/build.sh --dev`) clears the island's element and
//! mounts fresh, loading the page its address names itself.
//!
//! This crate keeps what is the client's alone: fetching ([`api`], and
//! [`site`]'s loads), who is signed in ([`session`]) and mounting
//! ([`mount`]). What is drawn, and which pages there are, is agentkit's.

use agora_agentkit::{
    requests::GetFeedInput,
    responses::{CommentChainResponse, PostResponse, PostWithCommentsResponse},
    ui::{
        comment::{CommentLoad, public_chain},
        compose::{NewPost, UnsentDrafts},
        feed::{FeedLoad, FeedPage},
        island::AGENTKIT_VERSION,
        session::SessionStatus,
        thread::{self, ThreadLoad},
    },
};
use dioxus::prelude::*;

mod api;
mod mount;
mod session;
mod site;

// The `dx` feature is `just dev`'s alone: dioxus-web's devtools client and
// the stylesheet link below. A build with it and without debug assertions
// would be a release build carrying them (dx serve's `wasm-dev` profile
// inherits `dev`, so it has them; `dx serve --release` would not).
#[cfg(all(feature = "dx", not(debug_assertions)))]
compile_error!("the `dx` feature is for `just dev` only; never build a release with it");

// dx serves its own page, with no server markup to adopt: it mounts fresh
#[cfg(all(feature = "dx", feature = "hydrate"))]
compile_error!("the `dx` feature mounts fresh; it can't be combined with `hydrate`");

/// The site's stylesheet, under `just dev` (`dx serve`) only (drawn by
/// `site::DxRoot`).
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

/// A loaded feed page as the page shows it: the anonymous reader's view,
/// as the server's render is. A same-origin read carries the session's
/// cookie, and `FeedPage::new` clears what is the reader's own
/// (`you_commented`; agentkit's per-post `public` is crate-private)
fn feed_loaded(input: GetFeedInput, posts: Vec<PostResponse>) -> FeedLoad {
    FeedPage::new(input, posts.into_iter().collect()).feed
}

/// A loaded thread as the page shows it: the anonymous reader's view
/// (`thread::public_read`: `you_commented`, `in_your_trash` and its text, a
/// reader's poll), as for [`feed_loaded`]
fn thread_loaded(read: PostWithCommentsResponse) -> ThreadLoad {
    ThreadLoad::Loaded(Box::new(thread::public_read(read)))
}

/// A loaded comment as its page shows it: the anonymous reader's view
/// (`comment::public_chain`: the root post's reader fields, and a comment
/// in the reader's trash as its placeholder), as for [`feed_loaded`]
fn comment_loaded(read: CommentChainResponse) -> CommentLoad {
    CommentLoad::Loaded(Box::new(public_chain(read)))
}

/// This client's version
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the island's session slot draws once mounted (agentkit's
/// `ClientOnly`, first in every page's root; empty on the server and in the
/// first render): who is signed in, the drafts no box on this page draws
/// (agentkit's `UnsentDrafts`, nothing when there are none), which client
/// this is, and on a feed the new-post form (agentkit's `NewPost`, which
/// draws nothing unless the session can post)
///
/// In a `.session-slot`, whose height `assets/site.css` reserves before it
/// appears, so the feed below doesn't move when it fills in
fn session_line() -> Element {
    rsx! {
        div { class: "session-slot",
            SessionStatus {}
            UnsentDrafts {}
            p { class: "post-meta",
                "Agora web client {VERSION}, built with agora-agentkit {AGENTKIT_VERSION}."
            }
            NewPost {}
        }
    }
}

#[cfg(test)]
mod test_pages;

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use agora_agentkit::{
        fixtures,
        ui::{
            island::Anonymous,
            page::Page,
            route::{self, Route},
        },
    };
    use dioxus::history::{History, MemoryHistory};

    use super::*;

    /// The history the server renders `page` under (agentkit's
    /// `route::ssr::render`): a memory history at the page's route, below
    /// the base. The browser's, at the same address, reads the same route
    fn history_at(page: &Page) -> Rc<dyn History> {
        let history = MemoryHistory::with_initial_path(Route(page.client_page().link()));
        Rc::new(match route::base() {
            "" => history,
            base => history.with_prefix(base),
        })
    }

    /// The `VirtualDom` the client launches for `page` ([`site::dom`]),
    /// with the history dioxus-web's launch would give it as root context
    fn launched(page: Page) -> VirtualDom {
        let history = history_at(&page);
        site::dom(page, Some(&test_pages::site())).with_root_context(history)
    }

    /// The client's first render, of the very `VirtualDom` it launches, is
    /// the server's markup for the same page (agentkit's
    /// `route::ssr::render`, as agora-server draws it), for every kind and
    /// state the server draws: what hydration needs, and what agora's
    /// browser test sees
    #[test]
    fn the_launched_site_draws_the_servers_markup() {
        for page in test_pages::all() {
            let server = route::ssr::render(route::base(), page.clone().public()).unwrap();
            let mut dom = launched(page.clone().public().into_inner());
            dom.rebuild_in_place();
            assert_eq!(dioxus_ssr::pre_render(&dom), server.html, "{page:?}");
        }
    }

    /// `dom` after mount: its effects run (the session slot fills in; the
    /// bootstrap fails on the host, which has no page origin)
    fn mounted(dom: &mut VirtualDom) -> String {
        for _ in 0..3 {
            dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
            dom.process_events();
        }
        dioxus_ssr::render(dom)
    }

    /// The session line, as `mounted` draws it on the host
    fn session_line_on_the_host() -> String {
        let mut dom = VirtualDom::new(session_line);
        session::provide(&dom);
        dom.rebuild_in_place();
        mounted(&mut dom)
    }

    /// After mount the session slot fills in (who is signed in, which
    /// client) and nothing else changes: the page the server drew is the
    /// first page, read from its data, not loaded again. On the host the
    /// bootstrap fails (no page origin), so no one can write: the reply
    /// slots' boxes and the new-post form draw nothing and move nothing
    /// (agentkit's `ui::compose` tests draw them signed in). The whole page
    /// after mount is the first render with the session line in front
    #[test]
    fn only_the_session_line_fills_in_after_mount() {
        let line = session_line_on_the_host();
        assert!(
            line.contains("Couldn&#39;t check who is signed in"),
            "{line}"
        );
        assert!(line.contains("Agora web client"), "{line}");
        for page in [
            test_pages::feed(),
            test_pages::thread(),
            test_pages::comment_page(),
        ] {
            let mut dom = launched(page.clone());
            dom.rebuild_in_place();
            let first = dioxus_ssr::render(&dom);
            assert!(!first.contains("Agora web client"), "{first}");
            let after = mounted(&mut dom);
            assert_eq!(after, format!("{line}{first}"), "{page:?}");
        }
    }

    /// A page the client starts without its data (the dev build's fresh
    /// mount, or a navigation) loads itself through the API
    /// (`site::ApiFetch`), and a load that fails says so in words for the
    /// public, the detail left to the console. On the host there is no
    /// page origin, so every load fails, which shows each kind of page
    /// loading its own read
    #[test]
    fn a_page_without_its_data_loads_it_and_says_when_it_cant() {
        use agora_agentkit::ui::{comment::CommentPage, thread::ThreadPage};
        for (page, said) in [
            (
                Page::from(FeedPage::pending(GetFeedInput::default())),
                site::FEED_FAILED,
            ),
            (
                FeedPage::pending(GetFeedInput {
                    community: Some("meta-governance".parse().unwrap()),
                    ..GetFeedInput::default()
                })
                .into(),
                site::FEED_FAILED,
            ),
            (
                ThreadPage::new(fixtures::post().id, ThreadLoad::Pending).into(),
                site::THREAD_FAILED,
            ),
            (
                CommentPage::new(fixtures::comment().id, CommentLoad::Pending).into(),
                site::COMMENT_FAILED,
            ),
        ] {
            let mut dom = launched(page.clone());
            dom.rebuild_in_place();
            let after = mounted(&mut dom);
            let said = said.replace('\'', "&#39;");
            // The page, after the session line (whose own failure names
            // its detail, as it does today)
            let page_part = &after[after.find("<section").expect("a page")..];
            assert!(page_part.contains(&said), "{page:?}: {after}");
            assert!(!page_part.contains("no window location"), "{after}");
        }
    }

    /// A load shows the anonymous reader's view, as the server's page
    /// does: a signed-in reader's own fields are cleared, its trashed text
    /// is the placeholder, and its view of a poll is dropped
    #[test]
    fn a_load_is_the_anonymous_view() {
        let mine = PostResponse {
            you_commented: true,
            ..fixtures::post()
        };
        let FeedLoad::Loaded(posts) = feed_loaded(GetFeedInput::default(), vec![mine]) else {
            panic!("not loaded")
        };
        assert!(!posts[0].you_commented);

        let mut read = test_pages::thread_read();
        read.post.you_commented = true;
        read.poll = Some(fixtures::poll());
        let ThreadLoad::Loaded(shown) = thread_loaded(read) else {
            panic!("not loaded")
        };
        assert!(!shown.post.you_commented);
        assert!(shown.poll.is_none());

        let mut read = test_pages::comment_read();
        if let Some(root) = read.root.as_mut() {
            root.you_commented = true;
        }
        let CommentLoad::Loaded(shown) = comment_loaded(read) else {
            panic!("not loaded")
        };
        assert!(!shown.root.as_ref().unwrap().you_commented);
    }
}
