//! SPIKE (feature `router`): the whole site behind the Dioxus router.
//!
//! The server pre-renders agentkit's `SiteRoot` (the router over
//! `Route(AgoraLink)`) at the request's path and embeds the page it read
//! (`Option<Page>`); this launches the same root over the browser's
//! history, with a `PageData` that answers the first page from the block
//! and fetches every page after a client-side navigation.

use std::rc::Rc;

use agora_agentkit::{
    ids::PostId,
    requests::GetFeedInput,
    ui::{
        ClientSlot, ClientSlots,
        feed::FeedLoad,
        island::{self, DATA_ID, ROOT_ID},
        page::Page,
        route::{Fetch, PageData, SiteRoot},
        thread::ThreadLoad,
    },
};
use dioxus::prelude::*;
/// `futures`' `LocalBoxFuture`, without the crate
type LocalBoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'a>>;

/// The API, for the pages after the first
struct ApiFetch;

impl Fetch for ApiFetch {
    fn feed(&self, input: GetFeedInput) -> LocalBoxFuture<'static, FeedLoad> {
        Box::pin(async move {
            web_sys::console::log_1(&format!("agora-web: fetching feed {input:?}").into());
            match crate::api::feed(&input).await {
                Ok(posts) => crate::feed_loaded(input, posts),
                Err(e) => FeedLoad::Failed(e.to_string()),
            }
        })
    }

    fn thread(&self, post: PostId) -> LocalBoxFuture<'static, ThreadLoad> {
        Box::pin(async move {
            web_sys::console::log_1(&format!("agora-web: fetching post {post}").into());
            match crate::api::thread(post).await {
                Ok(read) => crate::thread_loaded(read),
                Err(e) => ThreadLoad::Failed(e.to_string()),
            }
        })
    }
}

/// The base the site is mounted at, from `<meta name="agora-base">`
/// (`/agora` today, empty at the root)
fn base() -> String {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| {
            d.query_selector(r#"meta[name="agora-base"]"#)
                .ok()
                .flatten()
        })
        .and_then(|m| m.get_attribute("content"))
        .unwrap_or_default()
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

/// The site's `VirtualDom`: the router over the page data and the
/// client's slots and session, as the server's but for the fetch
pub fn dom(first: Option<Page>) -> VirtualDom {
    let dom = VirtualDom::new(SiteRoot)
        .with_root_context(PageData::client(first, ApiFetch))
        .with_root_context(ClientSlots::new().with(ClientSlot::Session, crate::session_line));
    crate::session::provide(&dom);
    dom
}

pub fn launch() {
    let first: Option<Page> = match data_block().as_deref().map(island::from_script_text) {
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
    let base = base();
    let history = Rc::new(KeepFragment(dioxus::web::WebHistory::new(
        (!base.is_empty()).then_some(base),
        true,
    )));
    dioxus::web::launch::launch_virtual_dom(
        dom(first),
        dioxus::web::Config::new()
            .rootname(ROOT_ID)
            .history(history),
    );
}

/// The browser's history with the `#fragment` left out of the route the
/// router reads. dioxus-web's `WebHistory` includes it, and the router
/// replaces the URL whenever `Route`'s `Display` differs from the route it
/// read: without this, hydrating `/post/<id>#comment-<id>` drops the
/// fragment and scrolls to the top (observed)
struct KeepFragment(dioxus::web::WebHistory);

impl dioxus::history::History for KeepFragment {
    fn current_route(&self) -> String {
        let route = self.0.current_route();
        match route.split_once('#') {
            Some((route, _)) => route.to_string(),
            None => route,
        }
    }
    fn current_prefix(&self) -> Option<String> {
        self.0.current_prefix()
    }
    fn can_go_back(&self) -> bool {
        self.0.can_go_back()
    }
    fn go_back(&self) {
        self.0.go_back()
    }
    fn can_go_forward(&self) -> bool {
        self.0.can_go_forward()
    }
    fn go_forward(&self) {
        self.0.go_forward()
    }
    fn push(&self, route: String) {
        self.0.push(route)
    }
    fn replace(&self, path: String) {
        self.0.replace(path)
    }
    fn external(&self, url: String) -> bool {
        self.0.external(url)
    }
    fn updater(&self, callback: std::sync::Arc<dyn Fn() + Send + Sync>) {
        self.0.updater(callback)
    }
    fn include_prevent_default(&self) -> bool {
        self.0.include_prevent_default()
    }
}
