//! SPIKE (router-real): the whole site behind the Dioxus router, with
//! agentkit's real `ui::route` (branch `ui/router`).
//!
//! The server pre-renders agentkit's `SiteRoot` at the request's route and
//! embeds the page it read (`Page`); this launches the same root over the
//! browser's history, with a `PageData` that answers the first page from
//! the block and fetches every page after a client-side navigation.
//! agentkit's `SiteRoot` wraps the history itself (no rewrite of the
//! address, fragment kept), so the client gives dioxus-web's `WebHistory`
//! as is: with `Some(base)` always, never `None`, which would read
//! dioxus-cli-config's base path instead.

use std::rc::Rc;

use agora_agentkit::{
    ids::{CommentId, PostId},
    requests::GetFeedInput,
    ui::{
        ClientSlot, ClientSlots, DocumentTitle,
        feed::{FeedLoad, FeedPage},
        island::{self, DATA_ID, ROOT_ID},
        page::Page,
        route::{self, Fetch, PageData, SiteRoot},
        comment::CommentLoad,
        thread::ThreadLoad,
    },
};
use dioxus::prelude::*;
/// `futures`' `LocalBoxFuture`, without the crate
type LocalBoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'a>>;

/// The API, for the pages after the first
struct ApiFetch;

fn not_found(e: &crate::api::ApiError) -> bool {
    matches!(e, crate::api::ApiError::Status { status: 404, .. })
}

impl Fetch for ApiFetch {
    fn feed(&self, input: GetFeedInput) -> LocalBoxFuture<'static, FeedPage> {
        Box::pin(async move {
            web_sys::console::log_1(&format!("agora-web: fetching feed {input:?}").into());
            let feed = match crate::api::feed(&input).await {
                Ok(posts) => crate::feed_loaded(input.clone(), posts),
                Err(e) if not_found(&e) => FeedLoad::NotFound,
                Err(e) => FeedLoad::Failed(e.to_string()),
            };
            // SPIKE: no community header (the stand-in server has no
            // community read); the real client reads it with the feed
            FeedPage::new(input, feed)
        })
    }

    fn comment(&self, comment: CommentId) -> LocalBoxFuture<'static, CommentLoad> {
        Box::pin(async move {
            web_sys::console::log_1(&format!("agora-web: fetching comment {comment}").into());
            match crate::api::comment(comment).await {
                Ok(read) => CommentLoad::Loaded(Box::new(read)),
                Err(e) if not_found(&e) => CommentLoad::NotFound,
                Err(e) => CommentLoad::Failed(e.to_string()),
            }
        })
    }

    fn thread(&self, post: PostId) -> LocalBoxFuture<'static, ThreadLoad> {
        Box::pin(async move {
            web_sys::console::log_1(&format!("agora-web: fetching post {post}").into());
            match crate::api::thread(post).await {
                Ok(read) => crate::thread_loaded(read),
                Err(e) if not_found(&e) => ThreadLoad::NotFound,
                Err(e) => ThreadLoad::Failed(e.to_string()),
            }
        })
    }
}

/// The base the site is mounted at, from `<meta name="agora-base">`
fn base_meta() -> Option<String> {
    web_sys::window()?
        .document()?
        .query_selector(r#"meta[name="agora-base"]"#)
        .ok()
        .flatten()?
        .get_attribute("content")
}

fn data_block() -> Option<String> {
    let selector = format!(r#"script#{DATA_ID}[type="application/json"]"#);
    web_sys::window()?
        .document()?
        .query_selector(&selector)
        .ok()
        .flatten()?
        .text_content()
}

/// The site's `VirtualDom`: the router over the page data, the client's
/// slots, the session and the title hook
pub fn dom(first: Page) -> VirtualDom {
    let dom = VirtualDom::new(SiteRoot)
        .with_root_context(PageData::client(first, ApiFetch))
        .with_root_context(ClientSlots::new().with(ClientSlot::Session, crate::session_line))
        .with_root_context(DocumentTitle::new(|title| {
            if let Some(document) = web_sys::window().and_then(|w| w.document()) {
                document.set_title(title);
            }
        }));
    crate::session::provide(&dom);
    dom
}

pub fn launch() {
    let first: Page = match data_block().as_deref().map(island::from_script_text) {
        Some(Ok(first)) => first,
        Some(Err(e)) => {
            web_sys::console::warn_1(&format!("agora-web: {e}; not starting").into());
            return;
        }
        None => {
            web_sys::console::warn_1(&"agora-web: no data block; not starting".into());
            return;
        }
    };
    let base = base_meta().unwrap_or_default();
    if base != route::base() {
        web_sys::console::warn_1(
            &format!(
                "agora-web: the page says the site is at {base:?}, this client's agentkit at {:?}; not starting",
                route::base()
            )
            .into(),
        );
        return;
    }
    #[cfg(feature = "hydrate")]
    if let Err(e) = crate::mount::set_globals() {
        web_sys::console::warn_1(&format!("agora-web: {e}; not starting").into());
        return;
    }
    #[cfg(not(feature = "hydrate"))]
    if let Some(root) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(ROOT_ID))
    {
        root.set_inner_html("");
    }
    // `Some` even for "" (the root): `None` reads dioxus-cli-config's base
    let history = Rc::new(dioxus::web::WebHistory::new(Some(base), true));
    dioxus::web::launch::launch_virtual_dom(
        dom(first),
        dioxus::web::Config::new()
            .rootname(ROOT_ID)
            .history(history),
    );
}
