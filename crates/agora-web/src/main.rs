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
//! Every island is a page (agentkit's `ui::page::Page`: a feed or a
//! thread), drawn by one root, `PageIsland`; see the README, "Pages".
//!
//! Two builds ([`mount`]): the release build (`hydrate` feature,
//! `scripts/build.sh`) hydrates, and stays off a page it can't hydrate; the
//! dev build (`scripts/build.sh --dev`) clears the island's element and
//! mounts fresh, loading the page its address names itself.
//!
//! This crate keeps what is the client's alone: fetching ([`api`]), who is
//! signed in ([`session`]) and mounting ([`mount`]). What is drawn is
//! agentkit's.

use agora_agentkit::{
    ids::PostId,
    requests::GetFeedInput,
    responses::{PostResponse, PostWithCommentsResponse},
    ui::{
        ClientSlot, ClientSlots, Reload,
        feed::{FeedLoad, FeedPage},
        island::{AGENTKIT_VERSION, ROOT_ID},
        page::{Page, PageIsland},
        thread::{self, ThreadLoad},
    },
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

/// A feed page's [`Reload`]: the page of the feed the server drew (its
/// `input`: the community, sort and offset), fetched again. The Refresh and
/// Try again links, and a fresh mount's first load
fn reload_feed(input: GetFeedInput) -> Reload<FeedLoad> {
    Reload::new(move || {
        let input = input.clone();
        Box::pin(async move {
            match api::feed(&input).await {
                Ok(posts) => feed_loaded(input, posts),
                Err(e) => FeedLoad::Failed(e.to_string()),
            }
        })
    })
}

/// A thread page's [`Reload`]: the post with its comments, read as the
/// server's page reads it ([`api::thread`]). Try again, and a fresh mount's
/// first load
fn reload_thread(post: PostId) -> Reload<ThreadLoad> {
    Reload::new(move || {
        Box::pin(async move {
            match api::thread(post).await {
                Ok(read) => thread_loaded(read),
                Err(e) => ThreadLoad::Failed(e.to_string()),
            }
        })
    })
}

/// A reloaded feed page as the page shows it: the anonymous reader's view,
/// as the server's render is. A same-origin read carries the session's
/// cookie, and `FeedPage::new` clears what is the reader's own
/// (`you_commented`; agentkit's per-post `public` is crate-private)
fn feed_loaded(input: GetFeedInput, posts: Vec<PostResponse>) -> FeedLoad {
    FeedPage::new(input, posts.into_iter().collect()).feed
}

/// A reloaded thread as the page shows it: the anonymous reader's view
/// (`thread::public_read`: `you_commented`, `in_your_trash` and its text, a
/// reader's poll), as for [`feed_loaded`]
fn thread_loaded(read: PostWithCommentsResponse) -> ThreadLoad {
    ThreadLoad::Loaded(Box::new(thread::public_read(read)))
}

/// The island's `VirtualDom`, exactly as [`launch`] mounts it: agentkit's
/// [`PageIsland`], the root the server pre-rendered, with `page` as its
/// props; the page's [`Reload`] ([`reload_feed`] or [`reload_thread`], by
/// the page's kind), the client-only slots and the session
/// ([`session::provide`]) in root context. One function, so the host test
/// that compares its first render with the server's markup tests what ships
/// (a wrapper or a context that changed the first render would fail it).
/// Under dx, [`DxRoot`] around it (dx mounts fresh, never hydrates).
///
/// Slots: [`ClientSlot::Session`] draws [`session_line`].
/// [`ClientSlot::Reply`] (under a thread's post and each comment) is left
/// empty until compose (plan step 6): agentkit draws an unfilled slot as an
/// empty placeholder, before and after mount, so it draws nothing and moves
/// nothing (tested below)
fn island_dom(page: Page) -> VirtualDom {
    let reload = match &page {
        Page::Feed(feed) => Reloads::Feed(reload_feed(feed.input.clone())),
        Page::Thread(thread) => Reloads::Thread(reload_thread(thread.post)),
    };
    #[cfg(not(feature = "dx"))]
    let dom = VirtualDom::new_with_props(PageIsland, page);
    #[cfg(feature = "dx")]
    let dom = VirtualDom::new_with_props(DxRoot, DxRootProps { page });
    let dom = match reload {
        Reloads::Feed(reload) => dom.with_root_context(reload),
        Reloads::Thread(reload) => dom.with_root_context(reload),
    }
    .with_root_context(ClientSlots::new().with(ClientSlot::Session, session_line));
    session::provide(&dom);
    dom
}

/// The one [`Reload`] a page's root reads, by the page's kind
enum Reloads {
    Feed(Reload<FeedLoad>),
    Thread(Reload<ThreadLoad>),
}

/// This client's version
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the island's session slot draws once mounted (agentkit's
/// `ClientOnly`, first in every page's root; empty on the server and in the
/// first render): who is signed in, and which client this is
///
/// In a `.session-slot`, whose height `assets/site.css` reserves before it
/// appears, so the feed below doesn't move when it fills in
fn session_line() -> Element {
    rsx! {
        div { class: "session-slot",
            session::SessionStatus {}
            p { class: "post-meta",
                "Agora web client {VERSION}, built with agora-agentkit {AGENTKIT_VERSION}."
            }
        }
    }
}

/// Launches the island ([`island_dom`]) into its element
fn launch(page: Page) {
    dioxus::web::launch::launch_virtual_dom(
        island_dom(page),
        dioxus::web::Config::new().rootname(ROOT_ID),
    );
}

/// `just dev`'s root: the island, and the stylesheet link dx's page lacks
/// (a server page has its own; see [`SITE_CSS`]). A wrapper rather than a
/// line in the island, so a build without the feature is the build it was:
/// the shared root, unwrapped, which is what hydration needs
#[cfg(feature = "dx")]
#[component]
fn DxRoot(page: Page) -> Element {
    rsx! {
        document::Stylesheet { href: SITE_CSS }
        // `PageIsland` has no hooks of its own (it picks the page's root,
        // which is a component), so it can be called here
        {PageIsland(page)}
    }
}

#[cfg(test)]
mod test_pages;

#[cfg(test)]
mod tests {
    use agora_agentkit::ui::{island::DATA_ID, ssr};

    use super::*;

    /// The client's first render, of the very `VirtualDom` it launches, is
    /// the server's markup for the same page, for every kind and state the
    /// server draws: what hydration needs, and what agora's browser test
    /// sees
    #[test]
    fn the_launched_island_draws_the_servers_markup() {
        for page in test_pages::all() {
            let server = ssr::render_island(DATA_ID, PageIsland, page.clone()).unwrap();
            let mut dom = island_dom(page.clone());
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
    /// client) and nothing else changes: on a thread, the reply slots stay
    /// empty (compose is a later step), so an unfilled slot draws nothing
    /// and moves nothing. The whole page after mount is the first render
    /// with the session line in front
    #[test]
    fn only_the_session_line_fills_in_after_mount() {
        let line = session_line_on_the_host();
        assert!(
            line.contains("Couldn&#39;t check who is signed in"),
            "{line}"
        );
        assert!(line.contains("Agora web client"), "{line}");
        for page in [test_pages::feed(), test_pages::thread()] {
            let mut dom = island_dom(page.clone());
            dom.rebuild_in_place();
            let first = dioxus_ssr::render(&dom);
            assert!(!first.contains("Agora web client"), "{first}");
            let after = mounted(&mut dom);
            assert_eq!(after, format!("{line}{first}"), "{page:?}");
        }
    }

    /// A reload shows the anonymous reader's view, as the server's page
    /// does: a signed-in reader's own fields are cleared, its trashed text
    /// is the placeholder, and its view of a poll is dropped
    #[test]
    fn a_reload_is_the_anonymous_view() {
        use agora_agentkit::fixtures;
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
    }

    /// A thread page's reload is the thread's, a feed page's the feed's,
    /// for the feed the page is (its community, sort and offset): the root
    /// reads the one its page needs
    #[test]
    fn each_page_gets_its_own_reload() {
        for (page, feed, thread) in [
            (test_pages::feed(), true, false),
            (test_pages::community(), true, false),
            (test_pages::thread(), false, true),
        ] {
            let dom = island_dom(page);
            dom.in_scope(ScopeId::ROOT, || {
                assert_eq!(try_consume_context::<Reload<FeedLoad>>().is_some(), feed);
                assert_eq!(
                    try_consume_context::<Reload<ThreadLoad>>().is_some(),
                    thread
                );
            });
        }
    }
}
