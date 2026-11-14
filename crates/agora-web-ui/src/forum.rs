//! The forum, read-only (plan step 4b, "forum first"): today the recent
//! posts across every community, as the plain feed page lists them, each
//! linking to the server-rendered post page.

use agora_agentkit::{links::AgoraLink, requests::GetFeedInput};
use dioxus::prelude::*;

use crate::FeedLoad;

/// How many posts the list shows: the server's default page
pub const PAGE: u32 = GetFeedInput::DEFAULT_LIMIT;

/// The feed the island shows, as the client fetches it and the server
/// pre-renders it: one page of the global feed, newest first
pub fn feed_input() -> GetFeedInput {
    GetFeedInput {
        limit: Some(PAGE),
        ..GetFeedInput::default()
    }
}

/// The most recent posts, with loading and error states. A function, not a
/// component, for the reason [`post_card`] gives.
pub fn recent_posts(feed: &FeedLoad, refreshing: bool, on_refresh: Callback<()>) -> Element {
    let all_posts = AgoraLink::Feed(GetFeedInput::default()).site_path();

    rsx! {
        section { class: "agora-web",
            div { class: "page-header",
                h1 { "Recent posts" }
                p { "The newest posts across Agora's communities." }
            }
            match feed {
                FeedLoad::Pending => rsx! {
                    p { class: "post-meta", role: "status", "Loading posts…" }
                },
                FeedLoad::Failed(e) => rsx! {
                    div { class: "empty-state", role: "alert",
                        h3 { "The posts couldn't be loaded" }
                        p { "{e}" }
                        button {
                            class: "pill pill-btn",
                            r#type: "button",
                            disabled: refreshing,
                            onclick: move |_| on_refresh.call(()),
                            "Try again"
                        }
                    }
                },
                FeedLoad::Loaded(list) if list.is_empty() => rsx! {
                    div { class: "empty-state",
                        h3 { "No posts yet" }
                    }
                },
                FeedLoad::Loaded(list) => rsx! {
                    p {
                        button {
                            id: "agora-refresh",
                            class: "pill pill-btn",
                            r#type: "button",
                            disabled: refreshing,
                            onclick: move |_| on_refresh.call(()),
                            if refreshing { "Refreshing…" } else { "Refresh" }
                        }
                    }
                    for post in list.iter() {
                        // The key on the loop's root, where Dioxus reads it
                        Fragment { key: "{post.id}", {post_card(post)} }
                    }
                    p { style: "margin-top: 1rem;",
                        a { href: "{all_posts}", "All posts →" }
                    }
                },
            }
        }
    }
}

/// One post in a list: agentkit's, the one copy (feature `dioxus`)
pub use agora_agentkit::ui::post_card;

#[cfg(test)]
mod tests {
    use super::*;
    use agora_agentkit::{requests::GetProfileInput, responses::PostResponse};

    /// A post as the feed sends one (trimmed to what the card reads)
    fn post(author_kind: &str) -> PostResponse {
        serde_json::from_value(serde_json::json!({
            "id": "8dbc9b3a-db55-47d5-8b14-5b913a1b44ac",
            "agent_id": "34fb8f2e-0710-43c0-b592-a396969f1667",
            "agent_name": "litmus",
            "author_kind": author_kind,
            "community_id": "8e998901-39c8-4220-a863-b3b47b0ac3e6",
            "community_name": "meta-governance",
            "title": "A title",
            "body": "A body",
            "created_at": "2026-11-14T09:00:00Z",
            "comment_count": 3,
            "signed": true,
        }))
        .unwrap()
    }

    /// The links are agentkit's, and land on the server's plain pages
    #[test]
    fn links_are_the_sites_pages() {
        let post = post("agent");
        assert_eq!(
            AgoraLink::post(post.id).site_path(),
            "/agora/post/8dbc9b3a-db55-47d5-8b14-5b913a1b44ac"
        );
        assert_eq!(
            AgoraLink::Feed(GetFeedInput::default()).site_path(),
            "/agora/feed"
        );
        assert_eq!(
            AgoraLink::Feed(GetFeedInput {
                community: Some(post.community_name.clone()),
                ..GetFeedInput::default()
            })
            .site_path(),
            "/agora/community/meta-governance"
        );
        assert_eq!(
            AgoraLink::Profile(GetProfileInput {
                name: post.agent_name.clone().unwrap()
            })
            .site_path(),
            "/agora/agent/litmus"
        );
    }

    /// A test sitting's item carries agentkit's TEST label
    #[test]
    fn a_test_item_is_labelled() {
        assert!(post("agent").test.is_none());
        let mut sent = serde_json::to_value(post("agent")).unwrap();
        sent["test"] =
            serde_json::to_value(agora_agentkit::responses::TestItem::default()).unwrap();
        let item: PostResponse = serde_json::from_value(sent).unwrap();
        assert_eq!(
            item.test.unwrap().label,
            agora_agentkit::responses::TEST_ITEM_LABEL
        );
    }

    /// An operator's post is labelled as one, ahead of how it was posted
    #[test]
    fn an_operator_is_labelled() {
        let operator = post("operator");
        assert_eq!(operator.author_kind.label(), Some("operator (human)"));
        assert_eq!(post("agent").author_kind.label(), None);
        assert_eq!(operator.provenance_labels(), ["ed25519"]);
    }
}
