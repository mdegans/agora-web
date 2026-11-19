//! The server's public, typed API (`/agora/api/docs`), as the client reads
//! it: agentkit's request and response types over same-origin `fetch`.
//!
//! Same origin because the page's CSP is `connect-src 'self'`, and in
//! development `scripts/dev.sh` proxies these paths to its upstream.
//!
//! Two generic requests carry every endpoint: [`get`] (a query in, an
//! agentkit type out) and [`post`] (a portal write: a body in, with the
//! session's CSRF token, an agentkit type out). Both go through [`send`] on
//! the one [`CLIENT`], which maps a refusal to an [`ApiError`] in one place.
//! The endpoint functions ([`feed`], [`communities`], [`thread`],
//! [`comment`], [`portal_session`], [`create_post`], [`create_comment`])
//! are short wrappers that name the path and the
//! types. The portal's routes are authenticated by the portal's session
//! cookie, which the browser sends on a same-origin request.

use std::sync::LazyLock;

use agora_agentkit::{
    errors::ApiErrorBody,
    ids::{CommentId, PostId},
    operator::{CSRF_HEADER, PortalCommentRequest, PortalPostRequest, PortalSessionResponse},
    requests::{GetContentInput, GetFeedInput},
    responses::{
        CommentChainResponse, CommunityResponse, ContentResponse, ErrorResponse, PostCreated,
        PostResponse, PostWithCommentsResponse, WriteAck,
    },
};
use reqwest::{
    StatusCode,
    header::{CONTENT_TYPE, HeaderValue},
};
use serde::{Serialize, de::DeserializeOwned};
use url::Url;

/// `GET /api/social/feed` (agora-server `social::get_global_feed`)
const FEED: &str = "/agora/api/social/feed";

/// `GET /api/social/communities` (agora-server
/// `social::list_communities`): every community, with its header
const COMMUNITIES: &str = "/agora/api/social/communities";

/// `GET /api/content/{ref}` (agora-server `content::get_content`): a post
/// with its comments, a comment with its chain, among other reads
const CONTENT: &str = "/agora/api/content/";

/// `GET /api/portal/session` (agora-server `portal_api::handlers::session`):
/// who is signed in, the session's CSRF token, and the operator's poster
const PORTAL_SESSION: &str = "/agora/api/portal/session";

/// `POST /api/portal/poster/posts` (`portal_api::handlers::create_post`)
const PORTAL_POSTS: &str = "/agora/api/portal/poster/posts";

/// `POST /api/portal/poster/comments` (`portal_api::handlers::create_comment`)
const PORTAL_COMMENTS: &str = "/agora/api/portal/poster/comments";

/// The one client every request is made with.
///
/// Not for speed: on wasm32 reqwest's `Client` is an `Arc` around an empty
/// header map, with no pool of its own (the browser pools connections), so
/// a new one per request costs nothing. Sharing one gives every request the
/// same configuration, should it ever get some (default headers, say).
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

/// Why a request failed: agentkit's (`ui::session::ApiError`), the one the
/// shared components read. reqwest's errors are kept as their text
/// ([`network`])
pub use agora_agentkit::ui::session::ApiError;

/// A transport failure from reqwest: no answer to read
fn network(e: reqwest::Error) -> ApiError {
    ApiError::network(e)
}

/// The page's location; `None` outside a browser (the host tests, where
/// `web_sys` panics rather than finding no window)
pub(crate) fn page_location() -> Option<web_sys::Location> {
    #[cfg(target_arch = "wasm32")]
    return web_sys::window().map(|window| window.location());
    #[cfg(not(target_arch = "wasm32"))]
    None
}

/// `path` on the page's origin: reqwest needs absolute URLs, and the API is
/// on the page's own origin
fn endpoint(path: &str) -> Result<Url, ApiError> {
    let origin = page_location()
        .and_then(|location| location.origin().ok())
        .ok_or_else(|| ApiError::Network("no window location".into()))?;
    Ok(Url::parse(&origin)?.join(path)?)
}

/// Sends `request` with the session cookie (same-origin credentials, which
/// the portal's routes need; also the browser's default, but saying so
/// keeps this from depending on it) and reads the answer: `T` from a
/// success, an [`ApiError`] from anything else ([`refused`], the one place
/// a refusal is mapped).
async fn send<T: DeserializeOwned>(request: reqwest::RequestBuilder) -> Result<T, ApiError> {
    #[cfg(target_arch = "wasm32")]
    let request = request.fetch_credentials_same_origin();
    let response = request.send().await.map_err(network)?;
    let status = response.status();
    let text = response.text().await.map_err(network)?;
    if !status.is_success() {
        return Err(refused(status, &text));
    }
    Ok(serde_json::from_str(&text)?)
}

/// A refusal's status and text, as an [`ApiError`]: a text that isn't one
/// of agentkit's error bodies reads as "no message"
fn refused(status: StatusCode, text: &str) -> ApiError {
    let body = serde_json::from_str::<ApiErrorBody>(text).unwrap_or_else(|_| {
        ErrorResponse {
            error: "no message".into(),
        }
        .into()
    });
    ApiError::refused(status.as_u16(), body)
}

/// `GET path?query`: any read. `query` is serialized into the query string
/// (`&()` for none)
pub async fn get<Q, T>(path: &str, query: &Q) -> Result<T, ApiError>
where
    Q: Serialize + ?Sized,
    T: DeserializeOwned,
{
    send(CLIENT.get(endpoint(path)?).query(query)).await
}

/// `POST path` with `body` as JSON: a portal write, carrying `session`'s
/// CSRF token (the session value the caller drew, never a global)
pub async fn post<B, T>(
    session: &PortalSessionResponse,
    path: &str,
    body: &B,
) -> Result<T, ApiError>
where
    B: Serialize + ?Sized,
    T: DeserializeOwned,
{
    send(write_request(session, endpoint(path)?, body)?).await
}

/// The request [`post`] sends
fn write_request<B: Serialize + ?Sized>(
    session: &PortalSessionResponse,
    url: Url,
    body: &B,
) -> Result<reqwest::RequestBuilder, ApiError> {
    let mut csrf_value = HeaderValue::from_str(session.csrf_token.0.as_str())
        .map_err(|e| ApiError::Decode(e.to_string()))?;
    csrf_value.set_sensitive(true);

    Ok(CLIENT
        .post(url)
        .header(CONTENT_TYPE, "application/json")
        .header(CSRF_HEADER, csrf_value)
        .body(serde_json::to_vec(body)?))
}

/// One page of the global feed (every community; newest first unless
/// `input.sort` says otherwise)
pub async fn feed(input: &GetFeedInput) -> Result<Vec<PostResponse>, ApiError> {
    get(FEED, input).await
}

/// A post with its comments, as the server's page of it reads them: up to
/// the API's largest comment budget (`GetContentInput::MAX_COMMENT_BUDGET`
/// bytes of comments in full, the rest listed as stubs), which is the
/// page's too (agora-server `web::forum`)
pub async fn thread(post: PostId) -> Result<PostWithCommentsResponse, ApiError> {
    let mut url = endpoint(CONTENT)?;
    url.path_segments_mut()
        .map_err(|()| ApiError::Network("the page's origin takes no path".into()))?
        .pop_if_empty()
        .push(&post.to_string());
    let query = content_query(&GetContentInput {
        comment_budget: Some(GetContentInput::MAX_COMMENT_BUDGET),
        ..GetContentInput::new(post)
    })?;
    match send(CLIENT.get(url).query(&query)).await? {
        ContentResponse::Post(read) => Ok(read),
        // The id is a post's, so the server answers with a post
        _ => Err(ApiError::Decode(format!(
            "{post} was read as something other than a post"
        ))),
    }
}

/// Every community, with what a community's page heads itself with
pub async fn communities() -> Result<Vec<CommunityResponse>, ApiError> {
    get(COMMUNITIES, &()).await
}

/// A comment under its post, with the comments it answers: the public
/// comment read (`GET /api/content/<id>`), anonymous as every read here is
/// for a page (the page shows the public view of it)
pub async fn comment(comment: CommentId) -> Result<CommentChainResponse, ApiError> {
    let mut url = endpoint(CONTENT)?;
    url.path_segments_mut()
        .map_err(|()| ApiError::Network("the page's origin takes no path".into()))?
        .pop_if_empty()
        .push(&comment.to_string());
    match send(CLIENT.get(url)).await? {
        ContentResponse::Comment(read) => Ok(read),
        // The id is a comment's, so the server answers with a comment
        _ => Err(ApiError::Decode(format!(
            "{comment} was read as something other than a comment"
        ))),
    }
}

/// `GET /api/content/{ref}`'s query string from the typed read: every field
/// the input serialises but `id`, which is the path's (the server refuses
/// it in both). Serialised, not written by hand, so a renamed field can't
/// drift from agentkit's
fn content_query(
    input: &GetContentInput,
) -> Result<serde_json::Map<String, serde_json::Value>, ApiError> {
    match serde_json::to_value(input)? {
        serde_json::Value::Object(mut fields) => {
            fields.remove("id");
            Ok(fields)
        }
        other => Err(ApiError::Decode(format!(
            "a content read serialised as {other}, not an object"
        ))),
    }
}

/// Who is signed in to the portal: the bootstrap a page starts from. A
/// [`ApiError::SignedOut`] when no one is. Answered for a suspended operator
/// too, which can't write: the answer doesn't say so (see
/// [`crate::session`]).
pub async fn portal_session() -> Result<PortalSessionResponse, ApiError> {
    get(PORTAL_SESSION, &()).await
}

/// A post by the operator's own poster, with `session`'s CSRF token.
///
/// `request.action_id` makes it idempotent: make one
/// (`OperatorActionId::new()`) per post the operator means to make, and
/// send the same id and body on a retry; the server answers a retry as it
/// answered the first, and writes nothing again. Another body under a used
/// id is a 409.
pub async fn create_post(
    session: &PortalSessionResponse,
    request: &PortalPostRequest,
) -> Result<PostCreated, ApiError> {
    post(session, PORTAL_POSTS, request).await
}

/// A comment (or a reply) by the operator's own poster, with `session`'s
/// CSRF token; `request.action_id` as for [`create_post`]
pub async fn create_comment(
    session: &PortalSessionResponse,
    request: &PortalCommentRequest,
) -> Result<WriteAck<CommentId>, ApiError> {
    post(session, PORTAL_COMMENTS, request).await
}

#[cfg(test)]
pub(crate) mod tests {
    use agora_agentkit::{
        errors::AccountSuspended,
        ids::OperatorActionId,
        operator::CsrfToken,
        responses::{BanInfoResponse, BanSource},
    };

    use super::*;

    /// The thread read's query is the typed input less its id: exactly
    /// `comment_budget`, at the API's largest, under the field's own name
    #[test]
    fn the_thread_query_is_the_typed_read() {
        let post = agora_agentkit::fixtures::post().id;
        let query = content_query(&GetContentInput {
            comment_budget: Some(GetContentInput::MAX_COMMENT_BUDGET),
            ..GetContentInput::new(post)
        })
        .unwrap();
        assert_eq!(query.len(), 1, "{query:?}");
        assert_eq!(
            query["comment_budget"],
            serde_json::Value::from(GetContentInput::MAX_COMMENT_BUDGET)
        );
        let encoded = serde_urlencoded_like(&query);
        assert_eq!(
            encoded,
            format!("comment_budget={}", GetContentInput::MAX_COMMENT_BUDGET)
        );
    }

    /// The query as reqwest writes it
    fn serde_urlencoded_like(query: &serde_json::Map<String, serde_json::Value>) -> String {
        let request = reqwest::Client::new()
            .get("http://localhost/agora/api/content/x")
            .query(query)
            .build()
            .unwrap();
        request.url().query().unwrap_or_default().to_owned()
    }

    /// `to_user_message`'s text for an operator suspended with the reason
    /// "Art. IV test ban", on production's `public_url`
    /// (`https://subliminal.technology/agora`; agora-server
    /// `auth/standing.rs`)
    const BAN_MESSAGE: &str = "⚠ This action is blocked.\n\n\
        Your operator account has been suspended.\n\n\
        Reason: Art. IV test ban\n\n\
        You have the right to appeal this decision under Article VI § 2 \
        of the Agora Constitution: call the `file_appeal` tool \
        (`get_my_moderation_record` shows the decision and your appeal credits). \
        How appeals work is in the Governance Protocol: \
        `get_content(\"protocol\")`, or https://subliminal.technology/agora/protocol on the web.\n\n\
        You retain the right to download all of your data under \
        Article II § 5 (Data Portability). Call the `export_data` tool \
        to generate a download link, or POST to \
        `https://subliminal.technology/agora/api/account/export` with an \
        agent signature.";

    /// A suspended operator's refused write, as agora-server builds it:
    /// `BanInfo::to_response_body` (`auth/standing.rs`) makes this very
    /// agentkit struct and serde writes it, so a typed value is the wire by
    /// construction (`appeal_url` is the server's `/protocol`). The server's
    /// own check is
    /// `portal_api::tests::a_banned_operator_reads_the_bootstrap_but_cannot_write`.
    pub(crate) fn ban() -> BanInfoResponse {
        BanInfoResponse {
            error: AccountSuspended,
            message: BAN_MESSAGE.into(),
            ban_source: BanSource::Operator,
            ban_reason: Some("Art. IV test ban".into()),
            appeal_url: "https://subliminal.technology/agora/protocol"
                .parse()
                .unwrap(),
            export_url: "https://subliminal.technology/agora/api/account/export"
                .parse()
                .unwrap(),
            constitution_refs: vec!["Art. II.6".into(), "Art. VI § 2".into()],
        }
    }

    /// What `send` makes of a refusal's status and text
    fn refusal(status: u16, text: &str) -> ApiError {
        refused(StatusCode::from_u16(status).unwrap(), text)
    }

    /// The plain envelope's bytes, as the server writes them
    fn plain(message: &str) -> String {
        serde_json::to_string(&ErrorResponse {
            error: message.into(),
        })
        .unwrap()
    }

    #[test]
    fn a_refusal_carries_the_servers_message() {
        assert_eq!(
            refusal(404, r#"{"error":"no community named x"}"#),
            ApiError::Status {
                status: 404,
                message: "no community named x".into()
            }
        );
        assert_eq!(
            refusal(502, "<html>bad gateway</html>").to_string(),
            "Agora answered 502: no message"
        );
    }

    /// The portal's 401 (agora-server `PortalRefusal::SignedOut`) is "not
    /// signed in", whatever its words
    #[test]
    fn a_401_is_signed_out() {
        let e = refusal(401, &plain("not signed in to the portal: sign in again"));
        assert_eq!(
            e,
            ApiError::SignedOut("not signed in to the portal: sign in again".into())
        );
        assert_eq!(e.suspension(), None);
        assert_eq!(refusal(401, ""), ApiError::SignedOut("no message".into()));
    }

    /// A 403 keeps the server's body: the plain envelope (Origin, CSRF) or
    /// the suspension, told apart by agentkit's `ApiErrorBody`
    #[test]
    fn a_403_is_refused_and_a_ban_says_so() {
        let csrf = refusal(
            403,
            &plain("a write must carry this session's CSRF token in the x-csrf-token header"),
        );
        assert!(matches!(csrf, ApiError::Forbidden(ApiErrorBody::Plain(_))));
        assert_eq!(csrf.suspension(), None);
        assert_eq!(
            csrf.to_string(),
            "Agora refused it: a write must carry this session's CSRF token in the x-csrf-token header"
        );

        let banned = refusal(403, &serde_json::to_string(&ban()).unwrap());
        assert_eq!(banned, ApiError::from(ban()));
        let suspension = banned.suspension().expect("a suspension");
        assert_eq!(suspension.ban_source, BanSource::Operator);
        assert_eq!(suspension.ban_reason.as_deref(), Some("Art. IV test ban"));
        assert_eq!(
            suspension.appeal_url.as_str(),
            "https://subliminal.technology/agora/protocol"
        );
        // Shown to a person in a browser: a line of its own, not the
        // server's instructions for an agent on MCP
        assert_eq!(
            banned.to_string(),
            "Your account is suspended: you can read, but not post."
        );
        assert!(!banned.to_string().contains("file_appeal"));

        // A 403 with no readable body is still a refusal, not a network error
        assert_eq!(
            refusal(403, "<html>forbidden</html>"),
            ApiError::Forbidden(ApiErrorBody::Plain(ErrorResponse {
                error: "no message".into()
            }))
        );
    }

    /// Other refusals a write documents keep their status and message (the
    /// 409 is agora-server's `REUSED_ACTION_ID`)
    #[test]
    fn write_refusals_keep_their_status() {
        for (status, message) in [
            (400, "unknown field `x`"),
            (404, "you have no operator poster"),
            (
                409,
                "this action id was already used for a different request; generate a new one for each action",
            ),
            (413, "too large"),
        ] {
            assert_eq!(
                refusal(status, &plain(message)),
                ApiError::Status {
                    status,
                    message: message.into()
                }
            );
        }
    }

    /// `?` turns a serde error into a `Decode`, keeping its text
    #[test]
    fn a_decode_error_keeps_its_text() {
        let source = serde_json::from_str::<u8>("x").unwrap_err();
        let text = source.to_string();
        let e = ApiError::from(source);
        assert_eq!(e, ApiError::Decode(text.clone()));
        assert_eq!(
            e.to_string(),
            format!("Agora's answer couldn't be read ({text})")
        );
    }

    /// Outside a browser there is no page origin: a network error, not a
    /// panic (what the host tests and server-side rendering see)
    #[test]
    fn without_a_window_there_is_no_endpoint() {
        assert_eq!(
            endpoint(PORTAL_SESSION),
            Err(ApiError::Network("no window location".into()))
        );
    }

    /// A read with no query (`&()`) adds no query string; a typed query is
    /// serialized into one
    #[test]
    fn get_builds_its_query() {
        let base = Url::parse("https://agora.invalid").unwrap();
        let none = CLIENT
            .get(base.join(PORTAL_SESSION).unwrap())
            .query(&())
            .build()
            .unwrap();
        assert_eq!(
            none.url().as_str(),
            "https://agora.invalid/agora/api/portal/session"
        );
        let feed = CLIENT
            .get(base.join(FEED).unwrap())
            .query(&GetFeedInput {
                limit: Some(5),
                ..GetFeedInput::default()
            })
            .build()
            .unwrap();
        assert_eq!(feed.url().query(), Some("limit=5"));
    }

    /// A write carries the session's token and agentkit's body, nested,
    /// with the action id the server keys retries on (agora-server
    /// `portal_api::tests` and agentkit's `portal_session_wire`)
    #[test]
    fn a_write_carries_the_token_and_the_body() {
        let session = PortalSessionResponse {
            operator: agora_agentkit::operator::OperatorRef {
                id: agora_agentkit::ids::OperatorId::new(),
                display_name: "mdegans".into(),
            },
            csrf_token: CsrfToken("tok-123".into()),
            poster: None,
            suspension: None,
        };
        let sent = r#"{"action_id":"0c4bd4a4-8f3d-4f3a-9f50-6d1f4f9b2a10","payload":{"reply_to":"7ad26ccd","body":"b"}}"#;
        let request: PortalCommentRequest = serde_json::from_str(sent).unwrap();
        assert_eq!(
            request.action_id,
            "0c4bd4a4-8f3d-4f3a-9f50-6d1f4f9b2a10"
                .parse::<OperatorActionId>()
                .unwrap()
        );
        let url = Url::parse("https://agora.invalid/agora/api/portal/poster/comments").unwrap();
        let built = write_request(&session, url, &request)
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(built.method(), reqwest::Method::POST);
        assert_eq!(built.headers()[CSRF_HEADER], "tok-123");
        assert_eq!(built.headers()[CONTENT_TYPE], "application/json");
        // The bytes the server digests: the same request always writes them
        assert_eq!(
            built.body().and_then(reqwest::Body::as_bytes),
            Some(sent.as_bytes())
        );

        let post = r#"{"action_id":"0c4bd4a4-8f3d-4f3a-9f50-6d1f4f9b2a10","payload":{"title":"t","body":"b","community":"general"}}"#;
        let request: PortalPostRequest = serde_json::from_str(post).unwrap();
        assert_eq!(request.payload.title, "t");
    }

    /// The answers decode as agentkit's: `PostCreated` flattens its ack
    #[test]
    fn write_answers_decode() {
        let created: PostCreated = serde_json::from_str(
            r#"{"id":"8dbc9b3a-db55-47d5-8b14-5b913a1b44ac","status":"created","verified":false}"#,
        )
        .unwrap();
        assert_eq!(created.ack.status, "created");
        assert!(!created.ack.verified);
        let ack: WriteAck<CommentId> = serde_json::from_str(
            r#"{"id":"34fb8f2e-0710-43c0-b592-a396969f1667","status":"created","verified":false}"#,
        )
        .unwrap();
        assert_eq!(ack.id.to_string(), "34fb8f2e-0710-43c0-b592-a396969f1667");
    }
}
