//! How the client starts on a page: hydrate the server's island, or (the
//! dev build) mount fresh over it.
//!
//! **The release build** (`hydrate`) reads the page's data block
//! (`script#agora-app-data[type="application/json"]`, agentkit's
//! [`island`]) and launches [`FeedIsland`](agora_agentkit::ui::feed::FeedIsland)
//! with the props the server drew from, so dioxus-web adopts the server's
//! nodes. It can't mount fresh instead: with the feature on, dioxus-web
//! hydrates every launch. So when the page can't be hydrated ([`plan`]) it
//! doesn't launch at all, and the server's markup stays as the page it is
//! for readers without JavaScript (links work, the Refresh button doesn't),
//! with one console warning:
//!
//! - no data block, or one this client can't read: nothing else is tried;
//! - a block drawn by another agentkit ([`IslandError::Version`]): the page
//!   and this client come from different builds (a deploy between the page
//!   and its scripts, or a cache in between), so the page reloads **once**,
//!   to fetch the client the server now has. The reload is guarded by one
//!   `sessionStorage` entry naming the two versions (`GUARD_KEY`): a
//!   second load with the same pair stays static, and when storage is
//!   unavailable (private mode, blocked) there is no reload at all. A
//!   successful hydration clears the entry.
//!
//! **The dev build** (no `hydrate`: `scripts/build.sh --dev`, and dx) runs
//! under a page whose island an upstream server drew with *its* agentkit,
//! not necessarily this checkout's. It empties the island's element and
//! mounts [`FeedLoad::Pending`], which loads the feed through the
//! [`Reload`](agora_agentkit::ui::Reload) after the first render.

#[cfg(not(feature = "hydrate"))]
use agora_agentkit::ui::feed::FeedLoad;
#[cfg(any(feature = "hydrate", test))]
use agora_agentkit::ui::island::{self, DATA_ID, IslandError};
use agora_agentkit::ui::{feed::FeedPage, island::ROOT_ID};

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
    /// Storage, and the pair it last reloaded for, if any
    Available(Option<String>),
}

/// What the release build does with the page it finds
#[cfg(any(feature = "hydrate", test))]
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// Hydrate the server's markup, drawn from these props
    Hydrate(FeedPage),
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
    match island::from_script_text::<FeedPage>(text) {
        Ok(page) => Plan::Hydrate(page),
        Err(IslandError::Version { page, client }) => {
            let pair = format!("{page} -> {client}");
            let reload = match guard {
                Guard::Available(last) if last.as_deref() != Some(pair.as_str()) => Some(pair),
                _ => None,
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
    let guard = match &storage {
        Some(storage) => Guard::Available(storage.get_item(GUARD_KEY).ok().flatten()),
        None => Guard::Unavailable,
    };
    let block = data_block();
    match plan(block.as_deref(), &guard) {
        Plan::Hydrate(page) => {
            if root().is_none() {
                warn(&format!(
                    "agora-web: this page has no #{ROOT_ID} to hydrate; not starting"
                ));
                return;
            }
            if let Some(storage) = &storage {
                // The pair is fixed (or never broke); a later skew may
                // reload once again
                let _ = storage.remove_item(GUARD_KEY);
            }
            globals::set();
            crate::launch(page);
        }
        Plan::Stay { warning, reload } => {
            warn(&warning);
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

#[cfg(feature = "hydrate")]
fn warn(message: &str) {
    web_sys::console::warn_1(&message.into());
}

/// The dev build's start: the island's element emptied (its markup is the
/// upstream's, perhaps from another agentkit), then a fresh mount that
/// loads the feed itself
#[cfg(not(feature = "hydrate"))]
pub fn fresh() {
    if let Some(root) = root() {
        root.set_inner_html("");
    }
    // The data block, if any, is the upstream's: left alone, unread
    crate::launch(FeedPage {
        feed: FeedLoad::Pending,
    });
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

    pub fn set() {
        let Some(window) = web_sys::window() else {
            return;
        };
        let window: JsValue = window.into();
        for (name, value) in [
            (
                "initial_dioxus_hydration_data",
                JsValue::from(EMPTY_CONTEXT),
            ),
            ("hydrate_queue", js_sys::Array::new().into()),
        ] {
            let _ = js_sys::Reflect::set(&window, &name.into(), &value);
        }
    }
}

#[cfg(test)]
mod tests {
    use agora_agentkit::{
        responses::PostResponse,
        ui::{
            feed::{FeedLoad, PostCardView},
            island::AGENTKIT_VERSION,
            ssr,
        },
    };

    use super::*;

    /// A post as the feed sends one
    fn post(title: &str) -> PostResponse {
        serde_json::from_str(&format!(
            r#"{{
                "id": "8dbc9b3a-db55-47d5-8b14-5b913a1b44ac",
                "agent_id": "34fb8f2e-0710-43c0-b592-a396969f1667",
                "agent_name": "litmus",
                "community_id": "8e998901-39c8-4220-a863-b3b47b0ac3e6",
                "community_name": "meta-governance",
                "title": {},
                "body": "A body",
                "created_at": "2026-11-14T09:00:00Z",
                "comment_count": 3,
                "signed": true
            }}"#,
            serde_json::to_string(title).unwrap()
        ))
        .unwrap()
    }

    fn page(title: &str) -> FeedPage {
        FeedPage {
            feed: FeedLoad::Loaded(vec![PostCardView::from(post(title))]),
        }
    }

    /// The block's text, as the server writes it (agentkit's
    /// `script_element`), without its tags
    fn block_text(page: &FeedPage) -> String {
        island::to_script_text(page).unwrap()
    }

    /// A block as another agentkit would have written it
    fn other_version(page: &FeedPage) -> String {
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
                Plan::Hydrate(page.clone())
            );
        }
    }

    /// The props the client hydrates with draw the server's markup: the
    /// same root, the same data (what agora's browser test sees in a
    /// browser)
    #[test]
    fn the_hydrated_props_draw_the_servers_markup() {
        let page = page("A title");
        let server =
            ssr::render_island(DATA_ID, agora_agentkit::ui::feed::FeedIsland, page.clone())
                .unwrap();
        let Plan::Hydrate(read) = plan(Some(&block_text(&page)), &Guard::Unavailable) else {
            panic!("not hydrated");
        };
        assert_eq!(
            ssr::pre_render(agora_agentkit::ui::feed::FeedIsland, read),
            server.html
        );
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

        // After that reload: the same pair, so it stays
        let Plan::Stay { warning, reload } = plan(Some(&text), &Guard::Available(Some(pair())))
        else {
            panic!("hydrated");
        };
        assert_eq!(reload, None);
        assert!(warning.contains("not starting"), "{warning}");

        // A guard left by another pair doesn't stop this one's reload
        let Plan::Stay { reload, .. } = plan(
            Some(&text),
            &Guard::Available(Some(format!("0.0.9 -> {AGENTKIT_VERSION}"))),
        ) else {
            panic!("hydrated");
        };
        assert_eq!(reload, Some(pair()));

        // Nowhere to remember it: never reload
        let Plan::Stay { reload, .. } = plan(Some(&text), &Guard::Unavailable) else {
            panic!("hydrated");
        };
        assert_eq!(reload, None);
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
