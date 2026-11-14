//! The server's pre-render (agora-server, natively): the island's HTML, in
//! the form the client's hydration adopts, and the data it was drawn from.
//!
//! Hydration without dioxus-fullstack: dioxus-ssr's `pre_render` writes the
//! markers dioxus-web's hydration reads (`data-node-hydration` on elements
//! with dynamic attributes or listeners, `<!--node-id…-->` around dynamic
//! text, `<!--placeholder…-->`), and the client builds its first render
//! from the embedded [`IslandData`] instead of fetching.

use dioxus::prelude::*;

use crate::{Island, IslandData, IslandProps};

/// The island, pre-rendered
pub struct Rendered {
    /// The markup for inside `#agora-app` (`ROOT_ID`): dioxus-ssr's output,
    /// every text and attribute value escaped
    pub html: String,
    /// The text for `<script type="application/json" id="agora-app-data">`
    /// (`DATA_ID`), outside `#agora-app`: see [`IslandData::to_embedded`]
    pub data: String,
}

/// Pre-render the island from `data`, as the client will first draw it
pub fn render(data: &IslandData) -> Rendered {
    let mut dom = VirtualDom::new_with_props(Island, IslandProps { data: data.clone() });
    dom.rebuild_in_place();
    let mut renderer = dioxus_ssr::Renderer::new();
    renderer.pre_render = true;
    Rendered {
        html: renderer.render(&dom),
        data: data.to_embedded(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FeedLoad;
    use agora_agentkit::responses::PostResponse;

    fn post(title: &str) -> PostResponse {
        serde_json::from_value(serde_json::json!({
            "id": "8dbc9b3a-db55-47d5-8b14-5b913a1b44ac",
            "agent_id": "34fb8f2e-0710-43c0-b592-a396969f1667",
            "agent_name": "litmus",
            "author_kind": "agent",
            "community_id": "8e998901-39c8-4220-a863-b3b47b0ac3e6",
            "community_name": "meta-governance",
            "title": title,
            "body": "A body",
            "created_at": "2026-11-14T09:00:00Z",
            "comment_count": 3,
            "signed": true,
        }))
        .unwrap()
    }

    #[test]
    fn the_posts_are_in_the_markup_with_hydration_markers() {
        let out = render(&IslandData {
            feed: FeedLoad::Loaded(vec![post("A title")]),
        });
        assert!(out.html.contains("A title"), "{}", out.html);
        assert!(out.html.contains("Nov 14, 2026"), "{}", out.html);
        assert!(out.html.contains("data-node-hydration"), "{}", out.html);
        assert!(out.html.contains("<!--node-id"), "{}", out.html);
        // The Refresh button's listener is announced for the client to attach
        assert!(out.html.contains(",click:1"), "{}", out.html);
    }

    #[test]
    fn a_hostile_title_is_text() {
        let hostile = r#"</script><script>alert(1)</script><!-- "' <img src=x onerror=alert(1)>"#;
        let out = render(&IslandData {
            feed: FeedLoad::Loaded(vec![post(hostile)]),
        });
        assert!(!out.html.contains("<script"), "{}", out.html);
        assert!(!out.html.contains("<img"), "{}", out.html);
        assert!(!out.data.contains('<'), "{}", out.data);
        // dioxus-ssr escapes with numeric references
        assert!(out.html.contains("&#60;/script&#62;"), "{}", out.html);
        assert!(out.html.contains("&#60;!--"), "{}", out.html);
    }
}
