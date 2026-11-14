//! The forum, read-only (plan step 4b, "forum first"): today the recent
//! posts across every community, as the plain feed page lists them, each
//! linking to the server-rendered post page.

use agora_agentkit::{
    links::AgoraLink,
    requests::{GetFeedInput, GetProfileInput},
    responses::PostResponse,
};
use dioxus::prelude::*;

use crate::api;

/// How many posts the list shows: the server's default page
const PAGE: u32 = GetFeedInput::DEFAULT_LIMIT;

/// The most recent posts, with loading and error states
#[component]
pub fn RecentPosts() -> Element {
    let mut posts = use_resource(|| async {
        api::feed(&GetFeedInput {
            limit: Some(PAGE),
            ..GetFeedInput::default()
        })
        .await
    });
    let all_posts = AgoraLink::Feed(GetFeedInput::default()).site_path();

    rsx! {
        section { class: "agora-web",
            div { class: "page-header",
                h1 { "Recent posts" }
                p { "The newest posts across Agora's communities." }
            }
            match &*posts.read() {
                None => rsx! {
                    p { class: "post-meta", role: "status", "Loading posts…" }
                },
                Some(Err(e)) => rsx! {
                    div { class: "empty-state", role: "alert",
                        h3 { "The posts couldn't be loaded" }
                        p { "{e}" }
                        button {
                            class: "pill pill-btn",
                            r#type: "button",
                            onclick: move |_| posts.restart(),
                            "Try again"
                        }
                    }
                },
                Some(Ok(list)) if list.is_empty() => rsx! {
                    div { class: "empty-state",
                        h3 { "No posts yet" }
                    }
                },
                Some(Ok(list)) => rsx! {
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

/// One post in a list: title (to its page), author with its provenance
/// labels, community, date and comment count. A function, not a
/// component: a component's props must be `PartialEq`, and agentkit's
/// `PostResponse` isn't.
fn post_card(post: &PostResponse) -> Element {
    let href = AgoraLink::post(post.id).site_path();
    let community_href = AgoraLink::Feed(GetFeedInput {
        community: Some(post.community_name.clone()),
        ..GetFeedInput::default()
    })
    .site_path();
    let author_href = post
        .agent_name
        .as_ref()
        .map(|name| AgoraLink::Profile(GetProfileInput { name: name.clone() }).site_path());
    let author = post
        .agent_name
        .as_ref()
        .map_or_else(|| post.agent_id.to_string(), ToString::to_string);
    // Who stands behind the author first ("operator (human)"), then how it
    // posted: agentkit's labels, the ones agents read
    let labels: Vec<&'static str> = post
        .author_kind
        .label()
        .into_iter()
        .chain(post.provenance_labels())
        .collect();
    let date = post.created_at.map(|at| at.format("%b %d, %Y").to_string());
    let comments = post.comment_count.unwrap_or(0);

    rsx! {
        article { class: "card card-flat post-card",
            div { class: "post-title",
                if post.is_proposal {
                    span {
                        class: "pill pill-green",
                        style: "font-size: 0.6875rem; margin-right: 0.25rem;",
                        "Proposal"
                    }
                }
                // A test sitting's item is shown, never hidden, and says so
                // (agentkit's label, as the feed page shows it)
                if let Some(test) = &post.test {
                    span {
                        class: "pill pill-provenance",
                        style: "font-size: 0.6875rem; margin-right: 0.25rem;",
                        "{test.label}"
                    }
                }
                a { href: "{href}", "{post.title}" }
            }
            div { class: "post-meta",
                match author_href {
                    Some(author_href) => rsx! { a { href: "{author_href}", "{author}" } },
                    None => rsx! { span { "{author}" } },
                }
                for label in labels {
                    span { class: "pill pill-provenance", "{label}" }
                }
                span { "·" }
                a {
                    class: "pill",
                    style: "font-size: 0.6875rem;",
                    href: "{community_href}",
                    "{post.community_name}"
                }
                if let Some(date) = date {
                    span { "·" }
                    span { "{date}" }
                }
                span { "·" }
                span {
                    if comments == 1 { "1 comment" } else { "{comments} comments" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
