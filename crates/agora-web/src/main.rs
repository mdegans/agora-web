//! Agora's web client.
//!
//! The client is a set of Dioxus islands: agora-server renders each page
//! (askama) with the island pre-rendered inside `#agora-app` from the same
//! components (`agora-web-ui`), and this binary hydrates it. The server
//! embeds this crate's release build and serves it under
//! `/agora/app/assets/`; see the README for the pipeline.
//!
//! Two builds: the release build (`hydrate` feature, scripts/build.sh)
//! adopts the server's nodes; the dev build (`scripts/dev.sh`) clears
//! `#agora-app` and mounts fresh, because the page it is served under comes
//! from an upstream server whose components may not be this checkout's.

use std::rc::Rc;

use agora_web_ui::{DATA_ID, FeedLoad, FeedSource, Island, IslandData, IslandProps, ROOT_ID};
use dioxus::prelude::*;

mod api;

/// The data the server rendered the island from, if the page has it
fn embedded_data() -> Option<IslandData> {
    let text = web_sys::window()?
        .document()?
        .get_element_by_id(DATA_ID)?
        .text_content()?;
    match IslandData::from_embedded(&text) {
        Ok(data) => Some(data),
        Err(e) => {
            web_sys::console::error_1(&format!("the island's data couldn't be read: {e}").into());
            None
        }
    }
}

/// The island's feed, fetched again (its Refresh button)
fn feed_source() -> FeedSource {
    FeedSource(Rc::new(|| {
        Box::pin(async {
            match api::feed(&agora_web_ui::forum::feed_input()).await {
                Ok(posts) => FeedLoad::Loaded(posts),
                Err(e) => FeedLoad::Failed(e.to_string()),
            }
        })
    }))
}

fn launch_with(data: IslandData) {
    let dom =
        VirtualDom::new_with_props(Island, IslandProps { data }).with_root_context(feed_source());
    dioxus::web::launch::launch_virtual_dom(dom, dioxus::web::Config::new().rootname(ROOT_ID));
}

#[cfg(feature = "hydrate")]
fn main() {
    let Some(data) = embedded_data() else {
        // A page without the pre-render: leave what it shows alone
        web_sys::console::error_1(&"no pre-rendered island to hydrate".into());
        return;
    };
    hydration::bootstrap();
    launch_with(data);
}

#[cfg(not(feature = "hydrate"))]
fn main() {
    // The upstream's pre-render, if any, is drawn by its components, not
    // necessarily these: start from an empty root, with its data if any
    if let Some(root) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(ROOT_ID))
    {
        root.set_inner_html("");
    }
    let data = embedded_data().unwrap_or(IslandData {
        feed: FeedLoad::Pending,
    });
    launch_with(data);
}

/// dioxus-web's hydration without dioxus-fullstack, which would set two
/// globals in an inline script (refused by the page's CSP), set here:
///
/// - `initial_dioxus_hydration_data`: base64 of the CBOR-encoded server
///   data `use_server_future` and friends pass to the client
///   (dioxus-fullstack-core's `HydrationContext`). The island passes its
///   data in its own block, so this is the empty list: CBOR `0x80`, "gA==".
///   dioxus-web reads it with `atob` and aborts if it is missing.
/// - `hydrate_queue`: streamed suspense chunks that arrived before the
///   wasm; none here. dioxus-interpreter-js reads its `length` when hydration
///   starts and throws if it is missing.
#[cfg(feature = "hydrate")]
mod hydration {
    use web_sys::wasm_bindgen::JsValue;

    /// The empty hydration context, CBOR `[]` in base64
    const EMPTY_CONTEXT: &str = "gA==";

    pub fn bootstrap() {
        let window: JsValue = web_sys::window().expect("a window").into();
        let set = |name: &str, value: JsValue| {
            js_sys::Reflect::set(&window, &name.into(), &value).expect("window accepts a property");
        };
        set("initial_dioxus_hydration_data", EMPTY_CONTEXT.into());
        set("hydrate_queue", js_sys::Array::new().into());
    }
}
