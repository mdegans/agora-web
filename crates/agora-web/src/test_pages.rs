//! Pages for the host tests: one of each kind and state the server draws,
//! built from agentkit's fixtures (feature `test-fixtures`, a
//! dev-dependency only), so a new wire field is agentkit's to fill in.

use agora_agentkit::{
    fixtures,
    requests::GetFeedInput,
    responses::{
        CommentChainResponse, CommentResponse, CommunityResponse, PostResponse,
        PostWithCommentsResponse,
    },
    ui::{
        comment::{CommentLoad, CommentPage},
        feed::{FeedLoad, FeedPage},
        page::Page,
        thread::{ThreadLoad, ThreadPage},
    },
};

/// A body with a list, a link, a code block, a table and a blockquote
pub const MARKDOWN: &str = "Some *words*, with [a link](https://example.org/).\n\n\
    - one\n- two\n\n\
    ```rust\nfn main() {}\n```\n\n\
    | a | b |\n|---|---|\n| 1 | 2 |\n\n\
    > quoted\n";

/// A post titled `title`, with a distinct id from `n`
pub fn post(n: u128, title: &str) -> PostResponse {
    PostResponse {
        id: id(0x1000 + n),
        title: title.into(),
        body: MARKDOWN.into(),
        comment_count: Some(3),
        signed: Some(true),
        ..fixtures::post()
    }
}

/// An id ending in `n`'s hex digits (`n` below 2^48)
fn id<T: std::str::FromStr<Err: std::fmt::Debug>>(n: u128) -> T {
    format!("8dbc9b3a-db55-47d5-8b14-{n:012x}").parse().unwrap()
}

/// A comment `n`, replying to comment `parent` (or the post)
pub fn comment(n: u128, parent: Option<u128>, body: &str) -> CommentResponse {
    CommentResponse {
        id: id(0x2000 + n),
        parent_comment_id: parent.map(|p| id(0x2000 + p)),
        body: body.into(),
        // `n` minutes after the fixtures' time, so the thread's order is
        // the comments' (`n` below 60)
        created_at: serde_json::from_str(&format!(r#""2026-11-04T09:{n:02}:00Z""#)).unwrap(),
        ..fixtures::comment()
    }
}

/// The global feed's first page: two posts, one with a hostile title
pub fn feed() -> Page {
    FeedPage::new(
        GetFeedInput::default(),
        FeedLoad::Loaded(vec![
            post(1, "A title"),
            post(2, "</script><script>alert(1)</script><!--"),
        ]),
    )
    .with_total(60)
    .into()
}

/// A community's feed, its second page by activity, with its header
pub fn community() -> Page {
    let input = GetFeedInput {
        community: Some("meta-governance".parse().unwrap()),
        sort: Some(agora_agentkit::enums::FeedSort::Active),
        offset: Some(25),
        ..GetFeedInput::default()
    };
    FeedPage::new(input, FeedLoad::Loaded(vec![post(3, "In a community")]))
        .with_community(CommunityResponse {
            description: Some("Talk about <b>governance</b>".into()),
            is_governance: true,
            ..fixtures::community()
        })
        .with_total(30)
        .into()
}

/// The empty and failed feeds
pub fn other_feeds() -> Vec<Page> {
    vec![
        FeedPage::new(GetFeedInput::default(), FeedLoad::Loaded(vec![])).into(),
        FeedPage::new(
            GetFeedInput::default(),
            FeedLoad::Failed("Agora couldn't load the posts just now.".into()),
        )
        .into(),
    ]
}

/// A thread: a markdown body, nested comments (three deep, and a fourth
/// reply that the page draws at depth 3 too), a deleted comment, a stub
pub fn thread_read() -> PostWithCommentsResponse {
    PostWithCommentsResponse {
        post: post(9, "A thread"),
        comments: vec![
            comment(1, None, "Top level, with `code`."),
            comment(2, Some(1), MARKDOWN),
            comment(3, Some(2), "Third"),
            comment(4, Some(3), "Fourth"),
            CommentResponse {
                deleted_at: Some(fixtures::at()),
                body: "[deleted by its author]".into(),
                ..comment(5, None, "")
            },
            comment(6, Some(5), "A reply to a deleted comment"),
        ],
        comment_stubs: vec![fixtures::comment_stub()],
        omitted_comment_count: 1,
        thread_summary: Some("A *summary*.".into()),
        ..fixtures::post_with_comments()
    }
}

pub fn thread() -> Page {
    ThreadPage::loaded(thread_read()).into()
}

/// A locked thread, a removed post's, and a failed one
pub fn other_threads() -> Vec<Page> {
    let mut locked = thread_read();
    locked.post.notice = Some(
        serde_json::from_str(
            r#"{"text": "Closed for the sitting", "locked": true,
                "set_by_name": "mdegans", "at": "2026-11-04T09:00:00Z"}"#,
        )
        .unwrap(),
    );
    let mut removed = thread_read();
    removed.post.deleted_at = Some(fixtures::at());
    removed.post.removed_by = Some(agora_agentkit::enums::RemovedBy::Moderation);
    vec![
        ThreadPage::loaded(locked).into(),
        ThreadPage::loaded(removed).into(),
        ThreadPage::new(
            fixtures::post().id,
            ThreadLoad::Failed("Agora couldn't load the thread just now.".into()),
        )
        .into(),
    ]
}

/// The site the server says its threads and comments are on
pub fn site() -> url::Url {
    url::Url::parse("https://agora.example/agora").unwrap()
}

/// A comment's read: a reply under the thread's post, two comments up the
/// chain, with earlier ones left out
pub fn comment_read() -> CommentChainResponse {
    let read = thread_read();
    CommentChainResponse {
        post_id: read.post.id,
        post_title: Some(read.post.title.clone()),
        root: Some(read.post),
        omitted_ancestors: 2,
        chain: vec![
            comment(2, Some(1), MARKDOWN),
            comment(3, Some(2), "Third"),
            comment(
                4,
                Some(3),
                "Fourth, with [a link](https://agora.example/agora/feed).",
            ),
        ],
        ..fixtures::comment_chain()
    }
}

/// A comment's page, as the server draws it (on its site)
pub fn comment_page() -> Page {
    let read = comment_read();
    let id = read.chain.last().unwrap().id;
    CommentPage::loaded(id, read).at(&site()).into()
}

/// A comment on the post itself, and a failed comment page
pub fn other_comments() -> Vec<Page> {
    vec![
        CommentPage::loaded(fixtures::comment().id, fixtures::comment_chain()).into(),
        CommentPage::new(
            fixtures::comment().id,
            CommentLoad::Failed("Agora couldn't load this comment just now.".into()),
        )
        .into(),
    ]
}

/// Every page above
pub fn all() -> Vec<Page> {
    let mut pages = vec![
        feed(),
        community(),
        thread(),
        ThreadPage::loaded(thread_read()).at(&site()).into(),
        comment_page(),
    ];
    pages.extend(other_feeds());
    pages.extend(other_threads());
    pages.extend(other_comments());
    pages
}
