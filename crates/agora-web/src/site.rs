//! The site's pages behind the Dioxus router (agentkit's `ui::route`).
//!
//! The server pre-renders agentkit's [`SiteRoot`] at the request's route
//! and embeds the page it read ([`Page`]) in the data block. The client
//! launches the same root, [`dom`], with the same contexts the server's
//! render has and the ones only a client has:
//!
//! - the history (root context, from dioxus-web's launch): the browser's
//!   (`WebHistory`, under the base the page names) when hydrating, a
//!   memory history in the dev build ([`crate::mount`]). `SiteRoot` wraps
//!   it itself: the address bar keeps what was typed, `#fragment` and
//!   scroll included, and a page the client doesn't draw is a full load;
//! - [`PageData`]: the first page from the block, every page after a
//!   client-side navigation through [`ApiFetch`];
//! - the client's slots (the session line), the session, and
//!   [`DocumentTitle`], which sets `document.title` as pages change (the
//!   CSP blocks dioxus's own `document::Title`).
//!
//! Which pages the client draws is agentkit's one list
//! (`route::ClientPage::of`): the feeds, a post's thread, a comment's
//! page and the operator poster's inbox. Every other link is a plain
//! `<a>`, a full page load.

use std::rc::Rc;

use agora_agentkit::{
    ids::{CommentId, PostId},
    requests::GetFeedInput,
    ui::{
        ClientSlot, ClientSlots, DocumentTitle,
        comment::CommentLoad,
        compose::ReplyBox,
        feed::{FeedLoad, FeedPage},
        inbox::Inbox,
        island::ROOT_ID,
        page::Page,
        route::{Fetch, PageData, SiteRoot},
        thread::ThreadLoad,
    },
};
use dioxus::{history::History, prelude::*};

use crate::api::{self, ApiError};

/// `futures`' `LocalBoxFuture`, without the crate
type LocalBoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'a>>;

/// What a feed's page says when it can't be loaded (agora-server's words
/// for its own page, `web::forum::FEED_FAILED`)
pub const FEED_FAILED: &str = "Agora couldn't load the posts just now.";
/// What a thread's page says when it can't be loaded (agora-server's
/// `web::forum::THREAD_FAILED`)
pub const THREAD_FAILED: &str = "Agora couldn't load this thread just now.";
/// What a comment's page says when it can't be loaded
pub const COMMENT_FAILED: &str = "Agora couldn't load this comment just now.";

/// The pages after the first, over the public API, read as an anonymous
/// reader reads them (the server's pages are that reader's view too). A
/// missing community, post or comment is `NotFound`; any other failure is
/// `Failed` in words for the public, with the detail in the console
pub struct ApiFetch;

/// A 404 from the API: the community, post or comment isn't there
fn not_found(e: &ApiError) -> bool {
    matches!(e, ApiError::Status { status: 404, .. })
}

/// A failed load, said in the console with its detail; the page gets
/// `public`'s words
fn failed(what: &str, e: &ApiError, public: &str) -> String {
    console_warn(&format!("agora-web: couldn't load {what}: {e}"));
    public.to_owned()
}

impl Fetch for ApiFetch {
    fn feed(&self, input: GetFeedInput) -> LocalBoxFuture<'static, FeedPage> {
        Box::pin(async move { feed(input).await })
    }

    fn thread(&self, post: PostId) -> LocalBoxFuture<'static, ThreadLoad> {
        Box::pin(async move {
            match api::thread(post).await {
                Ok(read) => crate::thread_loaded(read),
                Err(e) if not_found(&e) => ThreadLoad::NotFound,
                Err(e) => ThreadLoad::Failed(failed(&format!("post {post}"), &e, THREAD_FAILED)),
            }
        })
    }

    fn comment(&self, comment: CommentId) -> LocalBoxFuture<'static, CommentLoad> {
        Box::pin(async move {
            match api::comment(comment).await {
                Ok(read) => crate::comment_loaded(read),
                Err(e) if not_found(&e) => CommentLoad::NotFound,
                Err(e) => {
                    CommentLoad::Failed(failed(&format!("comment {comment}"), &e, COMMENT_FAILED))
                }
            }
        })
    }
}

/// A feed's page: its posts, and for a community's feed the community's
/// header, as the server's page has it. The API answers a community that
/// doesn't exist with a 404, which is `NotFound`. A header that can't be
/// read fails the page, as on the server: a community's posts under no
/// header would read as the global feed. Read one after the other: the
/// feed's 404 decides, and the header is read only for a page to draw
async fn feed(input: GetFeedInput) -> FeedPage {
    let what = match &input.community {
        Some(name) => format!("the feed of {}", name.canonical()),
        None => "the feed".to_owned(),
    };
    let posts = match api::feed(&input).await {
        Ok(posts) => posts,
        Err(e) if not_found(&e) => return FeedPage::new(input, FeedLoad::NotFound),
        Err(e) => return FeedPage::new(input, FeedLoad::Failed(failed(&what, &e, FEED_FAILED))),
    };
    let community = match &input.community {
        None => None,
        Some(name) => match api::communities().await {
            Ok(all) => match all.into_iter().find(|c| c.name == *name) {
                Some(community) => Some(community),
                // The feed was read, so the community exists: a list
                // without it is out of step, and said so
                None => {
                    let e =
                        ApiError::Decode(format!("the community list has no {}", name.canonical()));
                    let words = failed(&format!("the header of {what}"), &e, FEED_FAILED);
                    return FeedPage::new(input, FeedLoad::Failed(words));
                }
            },
            Err(e) => {
                let words = failed(&format!("the header of {what}"), &e, FEED_FAILED);
                return FeedPage::new(input, FeedLoad::Failed(words));
            }
        },
    };
    let page = FeedPage::new(input.clone(), crate::feed_loaded(input, posts));
    match community {
        Some(community) => page.with_community(community),
        None => page,
    }
}

/// A warning in the browser's console; nothing off the browser (the host
/// tests)
fn console_warn(message: &str) {
    #[cfg(target_arch = "wasm32")]
    web_sys::console::warn_1(&message.into());
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("{message}");
}

/// The page's origin (`window.location.origin`), the site its pages are
/// on: a thread's or comment's absolute links to it are the site's own.
/// `None`, said in the console, when there is no window or it doesn't
/// parse
pub fn origin() -> Option<url::Url> {
    let Some(origin) = api::page_location().and_then(|l| l.origin().ok()) else {
        console_warn("agora-web: the page's origin is unreadable; no site for its links");
        return None;
    };
    match url::Url::parse(&origin) {
        Ok(site) => Some(site),
        Err(e) => {
            console_warn(&format!(
                "agora-web: the page's origin doesn't parse ({e}); no site for its links"
            ));
            None
        }
    }
}

/// The site's `VirtualDom`, exactly as [`launch`] mounts it: agentkit's
/// [`SiteRoot`] (under dx, [`DxRoot`] around it) with [`PageData`] (the
/// first page, then [`ApiFetch`]; on `site` when given), the client's
/// slots, the session and the title in root context. The history is
/// given as root context too: dioxus-web's launch does it from its
/// config, the host tests here. One function, so the test that compares
/// its first render with the server's tests what ships
pub fn dom(first: Page, site: Option<&url::Url>) -> VirtualDom {
    let data = PageData::client(first, ApiFetch);
    let data = match site {
        Some(site) => data.at(site),
        None => data,
    };
    #[cfg(not(feature = "dx"))]
    let dom = VirtualDom::new(SiteRoot);
    #[cfg(feature = "dx")]
    let dom = VirtualDom::new(DxRoot);
    let dom = dom
        .with_root_context(data)
        .with_root_context(
            ClientSlots::new()
                .with(ClientSlot::Session, crate::session_line)
                .with(ClientSlot::Reply, ReplyBox)
                .with(ClientSlot::Inbox, Inbox),
        )
        .with_root_context(DocumentTitle::new(set_title));
    crate::session::provide(&dom);
    dom
}

/// `document.title`, as each page's title changes (agentkit's
/// `DocumentTitle`: in an effect, so never in the first render). Nothing
/// off the browser (the host tests)
fn set_title(title: &str) {
    #[cfg(target_arch = "wasm32")]
    match web_sys::window().and_then(|w| w.document()) {
        Some(document) => document.set_title(title),
        None => console_warn("agora-web: no document to set the title of"),
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = title;
}

/// Launches the site ([`dom`]) into the island's element over `history`
pub fn launch(first: Page, history: Rc<dyn History>, site: Option<&url::Url>) {
    dioxus::web::launch::launch_virtual_dom(
        dom(first, site),
        dioxus::web::Config::new()
            .rootname(ROOT_ID)
            .history(history),
    );
}

/// The site's stylesheet, under `just dev` (`dx serve`) only: see
/// [`crate::SITE_CSS`]
#[cfg(feature = "dx")]
#[component]
fn DxRoot() -> Element {
    rsx! {
        document::Stylesheet { href: crate::SITE_CSS }
        SiteRoot {}
    }
}
