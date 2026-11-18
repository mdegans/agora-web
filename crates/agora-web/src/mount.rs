//! How the client starts on a page: hydrate the server's island, or (the
//! dev build) mount fresh over it.
//!
//! **The release build** (`hydrate`) reads the page's data block
//! (`script#agora-app-data[type="application/json"]`, agentkit's
//! [`island`]) as a [`Page`] (which page it is, and the data it was drawn
//! from) and launches the site's router over it ([`crate::site`]), so
//! dioxus-web adopts the server's nodes. It can't mount fresh instead: with the feature on, dioxus-web
//! hydrates every launch. So when the page can't be hydrated ([`plan`]) it
//! doesn't launch at all, and the server's markup stays as the page it is
//! for readers without JavaScript (links work; its buttons, which would do
//! nothing without the client, are disabled), with one console warning:
//!
//! - no data block, one this client can't read, or an island element with
//!   nothing in it to adopt: nothing else is tried;
//! - a block drawn by another agentkit ([`IslandError::Version`]): the page
//!   and this client come from different builds (a deploy between the page
//!   and its scripts, or a cache in between), so the page reloads **once**,
//!   to fetch the client the server now has. It is a plain
//!   `location.reload()`, which gets the current client because agora-server
//!   serves the bundle `no-cache` with an `ETag`. The reload is guarded by
//!   one `sessionStorage` entry (`GUARD_KEY`, naming the two versions for
//!   whoever reads it): while it is there, no load reloads again, whatever
//!   pair it finds (two server generations behind one origin would
//!   otherwise alternate forever), and when storage is unavailable or
//!   unreadable (private mode, blocked) there is no reload at all. A
//!   successful hydration clears the entry.
//!
//! **The dev build** (no `hydrate`: `scripts/build.sh --dev`, and dx) runs
//! under a page whose island an upstream server drew with *its* agentkit,
//! not necessarily this checkout's, so it reads neither the markup nor the
//! data block. It empties the island's element and mounts the page the
//! address names ([`fresh_page`]: one of the pages the client draws), not
//! loaded, which loads itself through the site's
//! [`Fetch`](agora_agentkit::ui::route::Fetch) after the first render.
//!
//! Both launch the site behind the router ([`crate::site`]). The release
//! build also checks the page's `<meta name="agora-base">` against its
//! agentkit's base ([`check_base`]) and stays, as above, when they differ.

#[cfg(any(feature = "hydrate", test))]
use agora_agentkit::ui::island::{self, DATA_ID, IslandError};
use agora_agentkit::ui::route::Route;
use agora_agentkit::{
    links::AgoraLink,
    ui::{
        comment::{CommentLoad, CommentPage},
        feed::FeedPage,
        island::ROOT_ID,
        page::Page,
        route::{self, ClientPage},
        thread::{ThreadLoad, ThreadPage},
    },
};
#[cfg(not(feature = "hydrate"))]
use dioxus::history::MemoryHistory;

/// The `sessionStorage` key of the reload guard: its value is the
/// `"<page's agentkit> -> <client's agentkit>"` pair last reloaded for
#[cfg(feature = "hydrate")]
const GUARD_KEY: &str = "agora-web.reloaded-for";

/// What the reload guard remembers
#[cfg(any(feature = "hydrate", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Guard {
    /// No storage to remember a reload in: never reload
    Unavailable,
    /// Storage, and the pair it last reloaded for, if it has reloaded since
    /// the last successful hydration
    Available(Option<String>),
}

/// What the release build does with the page it finds
#[cfg(any(feature = "hydrate", test))]
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// Hydrate the server's markup, drawn from this page (boxed: a page is
    /// much larger than a warning)
    Hydrate(Box<Page>),
    /// Leave the page as the server drew it, and say why once. `reload` is
    /// the guard's new value when the page should reload (once) first
    Stay {
        warning: String,
        reload: Option<String>,
    },
}

/// The plan for a page whose data block holds `block` (its text, `None`
/// when the page has none), given what the reload guard remembers
#[cfg(any(feature = "hydrate", test))]
pub fn plan(block: Option<&str>, guard: &Guard) -> Plan {
    let Some(text) = block else {
        return Plan::Stay {
            warning: format!(
                "agora-web: this page has no data block (script#{DATA_ID}); \
                 not starting, the page stays as the server drew it"
            ),
            reload: None,
        };
    };
    match island::from_script_text::<Page>(text) {
        Ok(page) => Plan::Hydrate(Box::new(page)),
        Err(IslandError::Version { page, client }) => {
            let pair = format!("{page} -> {client}");
            // One reload until a hydration succeeds: any entry stops it
            let reload = match guard {
                Guard::Available(None) => Some(pair),
                Guard::Available(Some(_)) | Guard::Unavailable => None,
            };
            let then = if reload.is_some() {
                "reloading once for the current client"
            } else {
                "not starting, the page stays as the server drew it"
            };
            Plan::Stay {
                warning: format!(
                    "agora-web: the page was drawn with agora-agentkit {page}, \
                     this client has {client}; {then}"
                ),
                reload,
            }
        }
        Err(e @ IslandError::Json(_)) => Plan::Stay {
            warning: format!("agora-web: {e}; not starting, the page stays as the server drew it"),
            reload: None,
        },
    }
}

/// The page's document; `None` outside a browser
fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

/// The island's element
fn root() -> Option<web_sys::Element> {
    document()?.get_element_by_id(ROOT_ID)
}

/// The release build's start: [`plan`] for this page, then hydrate, or
/// stay (and perhaps reload once)
#[cfg(feature = "hydrate")]
pub fn hydrate() {
    let storage = web_sys::window().and_then(|w| w.session_storage().ok().flatten());
    // Storage that can't be read can't be trusted to stop a second reload
    let guard = match storage.as_ref().map(|s| s.get_item(GUARD_KEY)) {
        Some(Ok(last)) => Guard::Available(last),
        Some(Err(_)) | None => Guard::Unavailable,
    };
    let block = data_block();
    match plan(block.as_deref(), &guard) {
        Plan::Hydrate(page) => {
            // Hydration adopts the server's nodes: with none there, it
            // would adopt nothing and draw nothing
            if !root().is_some_and(|root| root.first_element_child().is_some()) {
                stay(&format!(
                    "agora-web: this page's #{ROOT_ID} is missing or empty, nothing to \
                     hydrate; not starting"
                ));
                return;
            }
            // The router's links and history are below the base: one the
            // page doesn't name, or names differently, would send every
            // navigation somewhere else
            let base = match check_base(base_meta().as_deref()) {
                Ok(base) => base,
                Err(warning) => {
                    stay(&warning);
                    return;
                }
            };
            // The router reads the address: it must be the page drawn
            match address_plan(&location_path(), &page) {
                Address::Keep => {}
                Address::Replace(to) => {
                    if let Err(warning) = replace_address(&to) {
                        stay(&warning);
                        return;
                    }
                }
                Address::Stay(warning) => {
                    stay(&warning);
                    return;
                }
            }
            if let Err(e) = globals::set() {
                stay(&format!("agora-web: {e}; not starting"));
                return;
            }
            if let Some(storage) = &storage {
                // The pair is fixed (or never broke); a later skew may
                // reload once again
                let _ = storage.remove_item(GUARD_KEY);
            }
            // `Some` even for "" (the root): `None` would read
            // dioxus-cli-config's base path instead. Scroll restoration on
            let history = std::rc::Rc::new(dioxus::web::WebHistory::new(Some(base.into()), true));
            crate::site::launch(*page, history, crate::site::origin().as_ref());
        }
        Plan::Stay { warning, reload } => {
            stay(&warning);
            // Only once the guard is written: a reload that can't be
            // remembered could be the first of many
            if let (Some(pair), Some(storage)) = (reload, &storage)
                && storage.set_item(GUARD_KEY, &pair).is_ok()
                && let Some(window) = web_sys::window()
            {
                let _ = window.location().reload();
            }
        }
    }
}

/// What the release build does about the address it starts at, which the
/// router reads to know which page it is on
#[cfg(any(feature = "hydrate", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    /// The address is the page the server drew (as typed: its query in
    /// another order, its `#fragment`, are kept)
    Keep,
    /// An address the router doesn't read with a page it draws: the app's
    /// own page, `/agora/app`, is the global feed's first page. The address
    /// becomes that page's (`history.replaceState`, no load), or the router
    /// would load the address from the server, again and again
    Replace(String),
    /// The address names another page than the one drawn, or isn't on the
    /// site: hydrating would draw the wrong page over the server's, so the
    /// page stays as the server drew it, saying why
    Stay(String),
}

/// The plan for `path_and_query` (the address's path and query, as
/// `location` has them) given the `page` the server drew there
#[cfg(any(feature = "hydrate", test))]
pub fn address_plan(path_and_query: &str, page: &Page) -> Address {
    let base = route::base();
    let below = path_and_query
        .strip_prefix(base)
        .filter(|rest| rest.starts_with('/'));
    let Some(below) = below else {
        return Address::Stay(format!(
            "agora-web: {path_and_query} isn't under the site's base, {base:?}; not starting, \
             the page stays as the server drew it"
        ));
    };
    match below.parse::<Route>() {
        Ok(route) if route.page().as_ref() == Some(&page.client_page()) => Address::Keep,
        Ok(route) => Address::Stay(format!(
            "agora-web: the address is {route}, the page drawn is {}; not starting, the page \
             stays as the server drew it",
            page.client_page().link().site_path()
        )),
        Err(_) => Address::Replace(page.client_page().link().site_path()),
    }
}

/// The address's path and query, as the router will read them
#[cfg(feature = "hydrate")]
fn location_path() -> String {
    web_sys::window()
        .map(|w| w.location())
        .and_then(|l| Some(l.pathname().ok()? + &l.search().ok()?))
        .unwrap_or_default()
}

/// The address made `to` without a load (`history.replaceState`), said in
/// the console; or why not
#[cfg(feature = "hydrate")]
fn replace_address(to: &str) -> Result<(), String> {
    let was = location_path();
    let history = web_sys::window()
        .ok_or("agora-web: no window")?
        .history()
        .map_err(|_| "agora-web: no history to set the address with; not starting".to_owned())?;
    history
        .replace_state_with_url(&web_sys::wasm_bindgen::JsValue::NULL, "", Some(to))
        .map_err(|_| format!("agora-web: couldn't set the address to {to}; not starting"))?;
    web_sys::console::info_1(
        &format!("agora-web: {was} is drawn as {to}, the page it shows; the address is now that")
            .into(),
    );
    Ok(())
}

/// The base the page says the site is mounted at: its
/// `<meta name="agora-base" content="…">`, which agora-server writes from
/// agentkit's `route::base()`
#[cfg(feature = "hydrate")]
fn base_meta() -> Option<String> {
    document()?
        .query_selector(r#"meta[name="agora-base"]"#)
        .ok()
        .flatten()?
        .get_attribute("content")
}

/// The base to launch the router under: the page's (`meta`, `None` when
/// it has none), which must be this client's agentkit's `route::base()`.
/// Otherwise the warning to stay with: the router's links and history are
/// below the base, so a page and a client that disagree would send every
/// navigation to the wrong address
#[cfg(any(feature = "hydrate", test))]
pub fn check_base(meta: Option<&str>) -> Result<&'static str, String> {
    let ours = route::base();
    match meta {
        Some(given) if given == ours => Ok(ours),
        Some(given) => Err(format!(
            "agora-web: the page says the site is at {given:?}, this client's agentkit at \
             {ours:?}; not starting, the page stays as the server drew it"
        )),
        None => Err(
            "agora-web: this page doesn't say where the site is (meta agora-base); \
             not starting, the page stays as the server drew it"
                .to_owned(),
        ),
    }
}

/// The text of the page's data block: the element with [`DATA_ID`], and
/// only if it is a JSON script (an element of another kind with that id,
/// say from markup a page let through, is not the block)
#[cfg(feature = "hydrate")]
fn data_block() -> Option<String> {
    let selector = format!(r#"script#{DATA_ID}[type="application/json"]"#);
    document()?
        .query_selector(&selector)
        .ok()
        .flatten()?
        .text_content()
}

/// Not starting: say why, once, mark the island static (`data-agora-static`,
/// so the stylesheet reserves no space for a session line that won't come),
/// and disable its buttons, which would do nothing without the client (a
/// link still works). The page isn't hydrated, so changing its markup can't
/// misplace anything
#[cfg(feature = "hydrate")]
fn stay(message: &str) {
    use web_sys::wasm_bindgen::JsCast;
    web_sys::console::warn_1(&message.into());
    let Some(root) = root() else {
        return;
    };
    // The session slot will never fill: assets/site.css stops reserving its
    // space on a page marked so. A failure only costs that space, so it is
    // said in the console and the page goes on
    if root.set_attribute("data-agora-static", "").is_err() {
        web_sys::console::warn_1(&"agora-web: couldn't mark the island static".into());
    }
    let Ok(buttons) = root.query_selector_all("button") else {
        return;
    };
    for i in 0..buttons.length() {
        if let Some(button) = buttons
            .item(i)
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        {
            let _ = button.set_attribute("disabled", "");
        }
    }
}

/// The dev build's start: the island's element emptied (its markup is the
/// upstream's, perhaps from another agentkit), then a fresh mount of the
/// page the address names, which loads itself
#[cfg(not(feature = "hydrate"))]
pub fn fresh() {
    if let Some(root) = root() {
        root.set_inner_html("");
    }
    // The data block, if any, is the upstream's: left alone, unread
    let path = web_sys::window()
        .map(|w| w.location())
        .and_then(|l| Some(l.pathname().ok()? + &l.search().ok()?))
        .unwrap_or_default();
    // The page's origin is the site: a body's absolute links to it are its
    // own (`ThreadPage::at`). Without one, only relative links are
    let site = crate::site::origin();
    let (page, unread) = fresh_page(&path, site.as_ref());
    if let Some(warning) = unread {
        web_sys::console::warn_1(&warning.into());
    }
    crate::site::launch(page.clone(), fresh_history(&page), site.as_ref());
}

/// The dev build's history: a memory history at `page`'s route, below the
/// site's base. The dev pages live at the app's own address (`/agora/app`,
/// dx's and `scripts/dev.sh`'s), which no route names, so the browser's
/// history would have the router load that address from the server, again
/// and again. Links between the client's pages navigate in the page (the
/// address bar stays put, and the browser's Back leaves the dev page);
/// every other link is a plain `<a>`, a full load
#[cfg(not(feature = "hydrate"))]
fn fresh_history(page: &Page) -> std::rc::Rc<dyn dioxus::history::History> {
    let history = MemoryHistory::with_initial_path(Route(page.client_page().link()));
    std::rc::Rc::new(match route::base() {
        "" => history,
        base => history.with_prefix(base),
    })
}

/// The page a fresh mount draws at `path_and_query`, not loaded: one of
/// the pages the client draws (agentkit's one list, `ClientPage::of`: a
/// feed's page, `/agora/feed` or `/agora/community/<name>` with their
/// sort, offset and limit, `limit` passed on as given, the API clamps it;
/// a post's thread; a comment's page), and `/agora/app` (dx's page too)
/// the global feed's first page. Any other address draws the global feed
/// with a warning naming it, for the console: a page the server draws (a
/// short id, a governance entry, a profile), or an address the link parser
/// refuses (an unknown query key, an offset out of range). A fresh
/// community page loads its header with its posts. A thread's or
/// comment's page is on `site` (the page's origin), as the server's is on
/// its public URL
#[cfg_attr(feature = "hydrate", allow(dead_code))]
pub fn fresh_page(path_and_query: &str, site: Option<&url::Url>) -> (Page, Option<String>) {
    let global = || FeedPage::pending(FeedPage::input()).into();
    let path = path_and_query.split('?').next().unwrap_or_default();
    let base = route::base();
    let app = [base, "/app"].concat();
    // The app's own page (and dx's), or no page of the site's at all
    if path == app
        || path.starts_with(&[app.as_str(), "/"].concat())
        || !path.starts_with(&[base, "/"].concat())
    {
        return (global(), None);
    }
    let unread = |why: String| {
        Some(format!(
            "agora-web: {path_and_query} {why}; drawing the global feed"
        ))
    };
    match AgoraLink::from_site_path(path_and_query) {
        Some(Ok(link)) => match ClientPage::of(&link) {
            Some(ClientPage::Feed(input)) => (FeedPage::pending(input).into(), None),
            Some(ClientPage::Thread(post)) => {
                let page = ThreadPage::new(post, ThreadLoad::Pending);
                let page = match site {
                    Some(site) => page.at(site),
                    None => page,
                };
                (page.into(), None)
            }
            Some(ClientPage::Comment(comment)) => {
                let page = CommentPage::new(comment, CommentLoad::Pending);
                let page = match site {
                    Some(site) => page.at(site),
                    None => page,
                };
                (page.into(), None)
            }
            None => (
                global(),
                unread(format!("is {link}, a page the server draws")),
            ),
        },
        Some(Err(e)) => (global(), unread(format!("doesn't read as a link ({e})"))),
        None => (global(), None),
    }
}

/// dioxus-web's hydration without dioxus-fullstack, which sets two globals
/// in inline scripts (this page's CSP refuses those), set here instead.
/// Both are Dioxus internals (dioxus-web 0.7.10 `src/lib.rs`,
/// dioxus-interpreter-js 0.7.10 `hydrate.ts`), which is why the pin is exact
/// and agora's browser test is the check:
///
/// - `initial_dioxus_hydration_data`: base64 of the CBOR the server's
///   `use_server_future` and friends hand the client (dioxus-fullstack-core's
///   `HydrationContext`). The island hands over its data in its own block,
///   so this is the empty list, CBOR `0x80`. dioxus-web reads it with `atob`
///   and fails without it.
/// - `hydrate_queue`: streamed suspense chunks that arrived before the
///   wasm; none. dioxus-interpreter-js reads its `length` when hydration
///   starts and throws without it.
#[cfg(feature = "hydrate")]
mod globals {
    use web_sys::{js_sys, wasm_bindgen::JsValue};

    /// CBOR `[]` (one byte, `0x80`), in base64
    pub const EMPTY_CONTEXT: &str = "gA==";

    /// Sets both, or says which couldn't be: hydrating without them fails
    /// inside dioxus-web, so the caller doesn't launch
    pub fn set() -> Result<(), String> {
        let window: JsValue = web_sys::window().ok_or("no window")?.into();
        for (name, value) in [
            (
                "initial_dioxus_hydration_data",
                JsValue::from(EMPTY_CONTEXT),
            ),
            ("hydrate_queue", js_sys::Array::new().into()),
        ] {
            match js_sys::Reflect::set(&window, &name.into(), &value) {
                Ok(true) => {}
                _ => return Err(format!("couldn't set window.{name} for hydration")),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use agora_agentkit::{
        requests::GetFeedInput,
        ui::{
            feed::FeedLoad,
            island::{AGENTKIT_VERSION, Anonymous},
        },
    };
    use dioxus::prelude::VirtualDom;

    use super::*;
    use crate::test_pages;

    /// A feed page with a post titled `title`
    fn page(title: &str) -> Page {
        FeedPage::new(
            GetFeedInput::default(),
            FeedLoad::Loaded(vec![test_pages::post(1, title)]),
        )
        .into()
    }

    /// The block's text, as the server writes it (agentkit's
    /// `script_element`), without its tags
    fn block_text(page: &Page) -> String {
        island::to_script_text(&page.clone().public()).unwrap()
    }

    /// A block as another agentkit would have written it
    fn other_version(page: &Page) -> String {
        let text = block_text(page);
        let stamp = format!(r#""agentkit":"{AGENTKIT_VERSION}""#);
        assert!(text.contains(&stamp), "{text}");
        text.replace(&stamp, r#""agentkit":"0.1.0""#)
    }

    fn pair() -> String {
        format!("0.1.0 -> {AGENTKIT_VERSION}")
    }

    /// The server's block, hostile title and all, hydrates with the props
    /// it was written from
    #[test]
    fn the_servers_block_hydrates() {
        let hostile = r#"</script><script>alert(1)</script><!--"#;
        let page = page(hostile);
        for guard in [
            Guard::Unavailable,
            Guard::Available(None),
            Guard::Available(Some(pair())),
        ] {
            assert_eq!(
                plan(Some(&block_text(&page)), &guard),
                Plan::Hydrate(Box::new(page.clone()))
            );
        }
    }

    /// What the client launches from the server's block draws the server's
    /// markup, for every page: the page agora-server draws through the
    /// router (`route::ssr::render`), its block read by [`plan`], then the
    /// `VirtualDom` the launch mounts (`crate::site::dom`) under the history
    /// the browser's would read
    #[test]
    fn the_hydrated_site_draws_the_servers_markup() {
        use dioxus::history::{History, MemoryHistory};
        for page in test_pages::all() {
            let server = route::ssr::render(route::base(), page.clone().public()).unwrap();
            let Plan::Hydrate(read) =
                plan(Some(&server_block(&server.script)), &Guard::Unavailable)
            else {
                panic!("not hydrated: {page:?}");
            };
            let history = MemoryHistory::with_initial_path(agora_agentkit::ui::route::Route(
                read.client_page().link(),
            ))
            .with_prefix(route::base());
            let mut dom: VirtualDom = crate::site::dom(*read, Some(&test_pages::site()))
                .with_root_context(std::rc::Rc::new(history) as std::rc::Rc<dyn History>);
            dom.rebuild_in_place();
            assert_eq!(dioxus_ssr::pre_render(&dom), server.html, "{page:?}");
        }
    }

    /// The router starts at the page the server drew: at its address as
    /// typed (another order, a fragment); `/agora/app`, which no route
    /// names, becomes the page it shows; an address naming another page,
    /// or off the site, stays
    #[test]
    fn the_router_starts_at_the_page_drawn() {
        let feed = test_pages::feed();
        let thread = test_pages::thread();
        // The query in another order than the canonical one: the page the
        // server read from that same address
        let typed = "/agora/feed?offset=0&sort=active";
        let Some(Ok(agora_agentkit::links::AgoraLink::Feed(input))) =
            agora_agentkit::links::AgoraLink::from_site_path(typed)
        else {
            panic!("a feed's address")
        };
        let sorted: Page = FeedPage::pending(input).into();
        assert_eq!(address_plan(typed, &sorted), Address::Keep);
        assert_eq!(
            address_plan("/agora/feed?sort=active&offset=0", &sorted),
            Address::Keep
        );
        assert_eq!(address_plan("/agora/feed", &feed), Address::Keep);
        let Page::Thread(t) = &thread else { panic!() };
        assert_eq!(
            address_plan(&format!("/agora/post/{}", t.post), &thread),
            Address::Keep
        );
        for app in ["/agora/app", "/agora/app/", "/agora/app?x=1"] {
            assert_eq!(
                address_plan(app, &feed),
                Address::Replace("/agora/feed".into()),
                "{app}"
            );
        }
        let Address::Stay(warning) = address_plan("/agora/feed", &thread) else {
            panic!("a thread drawn at the feed's address")
        };
        assert!(warning.contains("not starting"), "{warning}");
        assert!(matches!(
            address_plan("/elsewhere/feed", &feed),
            Address::Stay(_)
        ));
        assert!(matches!(
            address_plan("/agorafeed", &feed),
            Address::Stay(_)
        ));
    }

    /// The page's base must be this client's: anything else, or none, is a
    /// page the client stays off, saying why
    #[test]
    fn the_router_starts_only_under_its_own_base() {
        assert_eq!(check_base(Some(route::base())), Ok(route::base()));
        for (meta, said) in [
            (None, "doesn't say where the site is"),
            (
                Some("/elsewhere"),
                "the page says the site is at \"/elsewhere\"",
            ),
            (Some(""), "the page says the site is at \"\""),
            (Some("/agora/"), "the page says the site is at \"/agora/\""),
        ] {
            let warning = check_base(meta).unwrap_err();
            assert!(warning.contains(said), "{meta:?}: {warning}");
            assert!(warning.contains("not starting"), "{meta:?}: {warning}");
        }
    }

    /// The data block element's text, as the browser's `textContent` reads it
    fn server_block(element: &str) -> String {
        let start = element.find('>').unwrap() + 1;
        element[start..element.rfind("</script>").unwrap()].to_owned()
    }

    /// A fresh mount draws the page its address names, not loaded, and
    /// says so when it can't read the address
    #[test]
    fn a_fresh_mount_draws_the_page_its_address_names() {
        let post = agora_agentkit::fixtures::post().id;
        assert_eq!(
            fresh_page(&format!("/agora/post/{post}"), None),
            (
                Page::Thread(ThreadPage::new(post, ThreadLoad::Pending)),
                None
            )
        );
        // A comment's page by its full id is the client's since agentkit
        // 0.170.0
        let comment = agora_agentkit::fixtures::comment().id;
        assert_eq!(
            fresh_page(&format!("/agora/comment/{comment}"), None),
            (
                Page::Comment(CommentPage::new(comment, CommentLoad::Pending)),
                None
            )
        );
        let (Page::Feed(feed), None) = fresh_page(
            "/agora/community/meta-governance?sort=active&offset=25&limit=10",
            None,
        ) else {
            panic!("not a feed");
        };
        assert_eq!(feed.feed, FeedLoad::Pending);
        // The page's own path, in its canonical order, `limit` passed on
        assert_eq!(
            feed.href(),
            "/agora/community/meta-governance?limit=10&offset=25&sort=active"
        );
        let global = Page::Feed(FeedPage::new(FeedPage::input(), FeedLoad::Pending));
        for path in [
            "/agora/app",
            "/agora/app/",
            "/agora/app/?x=1",
            "/agora/feed",
            "/",
            "",
        ] {
            assert_eq!(fresh_page(path, None), (global.clone(), None), "{path}");
        }
        for path in [
            "/agora/post/8dbc9b3a",
            "/agora/comment/8dbc9b3a",
            "/agora/feed?offset=99999999999",
            "/agora/feed?bogus=1",
            "/agora/gov",
        ] {
            let (page, warning) = fresh_page(path, None);
            assert_eq!(page, global, "{path}");
            let warning = warning.unwrap_or_else(|| panic!("{path}: no warning"));
            assert!(warning.contains(path), "{warning}");
            assert!(warning.contains("drawing the global feed"), "{warning}");
        }
    }

    /// A fresh mount's thread is on the page's origin, so a body's absolute
    /// links to it are the site's own; with no origin, on none
    #[test]
    fn a_fresh_thread_is_on_the_pages_origin() {
        let post = agora_agentkit::fixtures::post().id;
        let site = url::Url::parse("http://localhost:8089").unwrap();
        let (Page::Thread(page), None) = fresh_page(&format!("/agora/post/{post}"), Some(&site))
        else {
            panic!("not a thread");
        };
        assert_eq!(page, ThreadPage::new(post, ThreadLoad::Pending).at(&site));
        assert_eq!(page.site.as_ref(), Some(&site));
        let (Page::Thread(page), None) = fresh_page(&format!("/agora/post/{post}"), None) else {
            panic!("not a thread");
        };
        assert_eq!(page.site, None);
        // A feed has no site to carry
        let (Page::Feed(_), None) = fresh_page("/agora/feed", Some(&site)) else {
            panic!("not a feed");
        };
    }

    /// No block, or one that isn't this island's JSON: stay, no reload
    #[test]
    fn a_missing_or_unreadable_block_stays_without_reloading() {
        for block in [None, Some(""), Some("{"), Some(r#"{"agentkit":1}"#)] {
            let Plan::Stay { warning, reload } = plan(block, &Guard::Available(None)) else {
                panic!("{block:?} hydrated");
            };
            assert_eq!(reload, None, "{block:?}");
            assert!(warning.contains("not starting"), "{warning}");
        }
    }

    /// Another agentkit's page reloads once per pair of versions, and only
    /// when the reload can be remembered
    #[test]
    fn another_agentkits_page_reloads_once() {
        let text = other_version(&page("A title"));
        let Plan::Stay { warning, reload } = plan(Some(&text), &Guard::Available(None)) else {
            panic!("hydrated another agentkit's page");
        };
        assert_eq!(reload, Some(pair()));
        assert!(
            warning.contains("drawn with agora-agentkit 0.1.0"),
            "{warning}"
        );
        assert!(warning.contains("reloading once"), "{warning}");

        // After that reload: the guard is there, so it stays
        let Plan::Stay { warning, reload } = plan(Some(&text), &Guard::Available(Some(pair())))
        else {
            panic!("hydrated");
        };
        assert_eq!(reload, None);
        assert!(warning.contains("not starting"), "{warning}");

        // A guard left by another pair stops it too: one reload until a
        // hydration succeeds, whatever the versions
        let Plan::Stay { reload, .. } = plan(
            Some(&text),
            &Guard::Available(Some(format!("0.0.9 -> {AGENTKIT_VERSION}"))),
        ) else {
            panic!("hydrated");
        };
        assert_eq!(reload, None);

        // Nowhere to remember it: never reload
        let Plan::Stay { reload, .. } = plan(Some(&text), &Guard::Unavailable) else {
            panic!("hydrated");
        };
        assert_eq!(reload, None);
    }

    /// Pages from two server generations behind one origin, alternating
    /// (or a cache serving both): one reload in all, not one per load. The
    /// guard is driven as `hydrate` drives it
    #[test]
    fn alternating_versions_reload_once() {
        let one = other_version(&page("A title"));
        let two = one.replace(r#""agentkit":"0.1.0""#, r#""agentkit":"0.0.9""#);
        let mut stored: Option<String> = None;
        let mut reloads = 0;
        for load in 0..10 {
            let text = if load % 2 == 0 { &one } else { &two };
            if let Plan::Stay {
                reload: Some(pair), ..
            } = plan(Some(text), &Guard::Available(stored.clone()))
            {
                stored = Some(pair);
                reloads += 1;
            }
        }
        assert_eq!(reloads, 1);
        // A hydration clears the guard; the next skew may reload once again
        let Plan::Stay { reload, .. } = plan(Some(&one), &Guard::Available(None)) else {
            panic!("hydrated");
        };
        assert!(reload.is_some());
    }

    /// The version is checked before the props: a page of another agentkit
    /// whose props have another shape still says "another agentkit" (and
    /// reloads once), not "unreadable"
    #[test]
    fn another_shape_from_another_agentkit_is_a_version_skew() {
        let text = r#"{"agentkit":"0.1.0","props":{"posts":[]}}"#;
        let Plan::Stay { reload, .. } = plan(Some(text), &Guard::Available(None)) else {
            panic!("hydrated");
        };
        assert_eq!(reload, Some(pair()));
    }
}
