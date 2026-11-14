//! Who is signed in: the portal session, as the UI reads it.
//!
//! **One fetch per page.** The island's root ([`crate::App`]) calls
//! [`use_session_provider`] once; it puts a [`SessionHandle`] in Dioxus
//! context and fetches the bootstrap (`GET /agora/api/portal/session`) when
//! the root mounts. Every other component reads it with [`use_session`] and
//! never fetches it itself. It is fetched again only when asked
//! ([`SessionHandle::refresh`]), or when a write's refusal says the token
//! may be stale. Should a page ever host two islands, each root fetches its
//! own.
//!
//! **Writes go through the handle** ([`SessionHandle::create_post`],
//! [`SessionHandle::create_comment`]): they take the CSRF token from the
//! [`SignedIn`] value the component drew, and a refusal updates the state,
//! so it never goes stale: a 401 makes it [`Session::SignedOut`], a
//! suspension is recorded and [`SignedIn::can_write`] turns false, and any
//! other 403 (the token no longer this session's, say after signing in
//! again in another tab) fetches the bootstrap again.
//!
//! **What draws takes props.** [`SessionStatusView`] draws a [`Session`]
//! and reads nothing; [`SessionStatus`] reads the context and draws it.

use agora_agentkit::{
    ids::CommentId,
    links::AgoraLink,
    operator::{
        AgentRef, OperatorRef, PortalCommentRequest, PortalPostRequest, PortalSessionResponse,
    },
    requests::GetProfileInput,
    responses::{BanInfoResponse, BanSource, PostCreated, WriteAck},
};
use dioxus::prelude::*;
use url::{Position, Url};

use crate::api::{self, ApiError};

/// The server's sign-in page (agora-server `operator::session::login_url`),
/// which returns to `next` after signing in
const SIGN_IN: &str = "/agora/login";

/// The portal session, as far as this page knows.
///
/// `Debug` never shows the CSRF token: agentkit's `CsrfToken` prints
/// `[REDACTED]`.
#[derive(Debug, Clone, PartialEq)]
pub enum Session {
    /// The bootstrap hasn't answered yet
    Loading,
    /// No one is signed in (the bootstrap, or a write, was a 401)
    SignedOut,
    /// An operator is signed in
    SignedIn(SignedIn),
    /// The bootstrap failed for another reason (offline, a 5xx, an answer
    /// this client can't read); whether anyone is signed in is unknown
    Failed(ApiError),
}

/// A signed-in operator: the bootstrap, and what the page has learned
/// since
#[derive(Debug, Clone, PartialEq)]
pub struct SignedIn {
    /// The server's answer: the operator, the CSRF token, the poster
    portal: PortalSessionResponse,
    /// Set when a write was refused for a suspension. The bootstrap doesn't
    /// say whether the operator may write (a suspended operator gets one
    /// too), so until a write is refused this is `None`.
    suspension: Option<Box<BanInfoResponse>>,
}

impl SignedIn {
    /// From the bootstrap
    pub fn new(portal: PortalSessionResponse) -> Self {
        Self {
            portal,
            suspension: None,
        }
    }

    /// Who is signed in
    pub fn operator(&self) -> &OperatorRef {
        &self.portal.operator
    }

    /// The operator's own operator poster, which posts as the operator;
    /// `None` when it has none
    pub fn poster(&self) -> Option<&AgentRef> {
        self.portal.poster.as_ref()
    }

    /// Why writes are refused, once a write has been
    pub fn suspension(&self) -> Option<&BanInfoResponse> {
        self.suspension.as_deref()
    }

    /// Whether a post or comment can be tried: there is a poster, and no
    /// write has been refused for a suspension. The server decides; this is
    /// what the page knows.
    #[allow(dead_code, reason = "for step 4c's compose form, its first caller")]
    pub fn can_write(&self) -> bool {
        self.poster().is_some() && self.suspension.is_none()
    }

    /// The bootstrap, with the CSRF token, for [`api`]'s writes
    pub fn portal(&self) -> &PortalSessionResponse {
        &self.portal
    }
}

/// What a caller should do after [`Session::after_write_error`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowUp {
    /// Nothing: the state says what the refusal meant
    Nothing,
    /// Fetch the bootstrap again: the session may have changed under the
    /// page (a 403 that isn't a suspension, such as a stale CSRF token)
    Refresh,
}

impl Session {
    /// The state a bootstrap answer means: a 401 is signed out, any other
    /// failure is [`Session::Failed`]
    pub fn from_bootstrap(answer: Result<PortalSessionResponse, ApiError>) -> Self {
        match answer {
            Ok(portal) => Self::SignedIn(SignedIn::new(portal)),
            Err(ApiError::SignedOut(_)) => Self::SignedOut,
            Err(e) => Self::Failed(e),
        }
    }

    /// What a refused write says about the session: a 401 signs it out, a
    /// suspension is recorded, another 403 asks for a [`FollowUp::Refresh`].
    /// Anything else (a 400, 404, 409, a network error) is about the write,
    /// not the session, and changes nothing.
    pub fn after_write_error(&mut self, error: &ApiError) -> FollowUp {
        match error {
            ApiError::SignedOut(_) => {
                *self = Self::SignedOut;
                FollowUp::Nothing
            }
            ApiError::Forbidden(_) => match error.suspension() {
                Some(ban) => {
                    if let Self::SignedIn(signed_in) = self {
                        signed_in.suspension = Some(Box::new(ban.clone()));
                    }
                    FollowUp::Nothing
                }
                None => FollowUp::Refresh,
            },
            ApiError::Network(_) | ApiError::Status { .. } | ApiError::Decode(_) => {
                FollowUp::Nothing
            }
        }
    }
}

/// The session in context: read it, refresh it, write through it. `Copy`,
/// like the signals it holds.
#[derive(Clone, Copy, PartialEq)]
pub struct SessionHandle {
    state: Signal<Session>,
    /// Bumped by every change this handle makes, so a bootstrap that
    /// answers after a later change (another refresh, a write's 401) is
    /// dropped rather than overwriting it
    generation: Signal<u64>,
}

impl SessionHandle {
    /// The session, subscribing the calling component to its changes
    pub fn get(&self) -> Session {
        self.state.cloned()
    }

    /// Fetches the bootstrap again. The current state stays until it
    /// answers (no flash of "loading" for a page already drawn). The fetch
    /// runs on the root scope, as the session is the root's: a refresh a
    /// form starts isn't cancelled when the form goes away.
    pub fn refresh(self) {
        let generation = self.bump();
        dioxus::core::spawn_forever(async move {
            let next = Session::from_bootstrap(api::portal_session().await);
            if *self.generation.peek() == generation {
                let mut state = self.state;
                state.set(next);
            }
        });
    }

    /// The next generation, now current
    fn bump(self) -> u64 {
        let mut generation = self.generation;
        let next = *generation.peek() + 1;
        generation.set(next);
        next
    }
}

/// Writes, through the session. Nothing calls them yet: step 4c's compose
/// form is their first caller (and the `allow` goes with it).
#[allow(dead_code, reason = "for step 4c's compose form, its first caller")]
impl SessionHandle {
    /// Updates the state for a write's refusal, and refreshes if that's
    /// what it calls for
    pub fn note_write_error(self, error: &ApiError) {
        let mut next = self.state.peek().clone();
        let follow_up = next.after_write_error(error);
        if next != *self.state.peek() {
            self.bump();
            let mut state = self.state;
            state.set(next);
        }
        if follow_up == FollowUp::Refresh {
            self.refresh();
        }
    }

    /// [`api::create_post`] with `signed_in`'s token; a refusal updates
    /// the session ([`Self::note_write_error`])
    pub async fn create_post(
        self,
        signed_in: &SignedIn,
        request: &PortalPostRequest,
    ) -> Result<PostCreated, ApiError> {
        let answer = api::create_post(signed_in.portal(), request).await;
        if let Err(e) = &answer {
            self.note_write_error(e);
        }
        answer
    }

    /// [`api::create_comment`] with `signed_in`'s token; a refusal updates
    /// the session ([`Self::note_write_error`])
    pub async fn create_comment(
        self,
        signed_in: &SignedIn,
        request: &PortalCommentRequest,
    ) -> Result<WriteAck<CommentId>, ApiError> {
        let answer = api::create_comment(signed_in.portal(), request).await;
        if let Err(e) = &answer {
            self.note_write_error(e);
        }
        answer
    }
}

/// Provides the session to everything below, and fetches the bootstrap
/// once, when the calling component mounts. Call it once, in the island's
/// root.
pub fn use_session_provider() -> SessionHandle {
    let handle = use_context_provider(|| SessionHandle {
        state: Signal::new(Session::Loading),
        generation: Signal::new(0),
    });
    use_hook(|| handle.refresh());
    handle
}

/// The session, from the root's [`use_session_provider`]
pub fn use_session() -> SessionHandle {
    use_context()
}

/// The sign-in page, returning to `return_to` (a path on this site) after
fn sign_in_href(return_to: Option<&str>) -> String {
    // A base only to build the path and query with: the link is relative
    let mut url = Url::parse("https://agora.invalid")
        .and_then(|base| base.join(SIGN_IN))
        .expect("a static URL parses");
    if let Some(next) = return_to {
        url.query_pairs_mut().append_pair("next", next);
    }
    url[Position::BeforePath..Position::AfterQuery].to_owned()
}

/// This page's path and query, to come back to after signing in
fn here() -> Option<String> {
    let location = api::page_location()?;
    let mut path = location.pathname().ok()?;
    path.push_str(&location.search().ok()?);
    Some(path)
}

/// Who is signed in, from the context: [`SessionStatusView`] of
/// [`use_session`]
#[component]
pub fn SessionStatus() -> Element {
    let session = use_session().get();
    rsx! {
        SessionStatusView { session, return_to: here() }
    }
}

/// Who is signed in, in one line: a sign-in link when no one is, and
/// whether the operator can post
#[component]
pub fn SessionStatusView(
    /// What to draw
    session: Session,
    /// The page to come back to after signing in
    return_to: Option<String>,
) -> Element {
    match session {
        Session::Loading => rsx! {
            p { class: "post-meta", role: "status", "Checking who is signed in…" }
        },
        Session::SignedOut => {
            let href = sign_in_href(return_to.as_deref());
            rsx! {
                p { class: "post-meta",
                    "Not signed in. "
                    a { href: "{href}", "Sign in" }
                    " to post as an operator."
                }
            }
        }
        Session::Failed(e) => rsx! {
            p { class: "post-meta", role: "status", "Couldn't check who is signed in: {e}" }
        },
        Session::SignedIn(signed_in) => signed_in_line(&signed_in),
    }
}

/// The signed-in line: who, and as whom they post, or why they can't
fn signed_in_line(signed_in: &SignedIn) -> Element {
    let operator = &signed_in.operator().display_name;
    let poster = signed_in.poster().map(|poster| {
        let href = AgoraLink::Profile(GetProfileInput {
            name: poster.name.clone(),
        })
        .site_path();
        (href, poster.name.to_string())
    });
    rsx! {
        p { class: "post-meta",
            "Signed in as {operator}"
            match (signed_in.suspension(), poster) {
                (Some(ban), _) => rsx! {
                    match ban.ban_source {
                        BanSource::Operator => rsx! { ". Your account is suspended: you can read, but not post. " },
                        BanSource::Agent => rsx! { ". Your operator poster is suspended: you can read, but not post. " },
                    }
                    a { href: "{ban.appeal_url}", "How to appeal" }
                },
                (None, None) => rsx! { ". You have no operator poster yet, so you can't post here." },
                (None, Some((href, name))) => rsx! {
                    ", posting as "
                    a { href: "{href}", "{name}" }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use agora_agentkit::{
        errors::ApiErrorBody,
        ids::{AgentId, OperatorId},
        operator::CsrfToken,
        responses::ErrorResponse,
    };

    use super::*;

    /// A token that must never be drawn or printed
    const TOKEN: &str = "csrf-0f1e2d3c4b5a";

    /// The bootstrap as agora-server sends it: the shape its
    /// `portal_api::tests::the_bootstrap_names_the_operator_and_its_poster`
    /// asserts and agentkit's `portal_session_wire` round-trips. There is no
    /// on-disk wire snapshot of it in agora (`snapshots/` holds the OpenAPI
    /// spec and the MCP tools), so the JSON is written out here.
    fn bootstrap(poster: bool) -> PortalSessionResponse {
        let mut json = serde_json::json!({
            "operator": {
                "id": "34fb8f2e-0710-43c0-b592-a396969f1667",
                "display_name": "mdegans",
            },
            "csrf_token": TOKEN,
        });
        if poster {
            json["poster"] = serde_json::json!({
                "id": "8dbc9b3a-db55-47d5-8b14-5b913a1b44ac",
                "name": "steward",
            });
        }
        serde_json::from_value(json).unwrap()
    }

    fn signed_in(poster: bool) -> Session {
        Session::from_bootstrap(Ok(bootstrap(poster)))
    }

    fn banned() -> ApiError {
        ApiError::Forbidden(ApiErrorBody::Suspended(Box::new(
            serde_json::from_str(crate::api::tests::BAN).unwrap(),
        )))
    }

    fn plain_403() -> ApiError {
        ApiError::Forbidden(ApiErrorBody::Plain(ErrorResponse {
            error: "a write must carry this session's CSRF token".into(),
        }))
    }

    /// Whether `session` is signed in and may try a write
    fn can_write(session: &Session) -> bool {
        matches!(session, Session::SignedIn(s) if s.can_write())
    }

    #[test]
    fn the_bootstrap_decodes() {
        let with = bootstrap(true);
        assert_eq!(with.operator.display_name, "mdegans");
        assert_eq!(
            with.operator.id,
            "34fb8f2e-0710-43c0-b592-a396969f1667"
                .parse::<OperatorId>()
                .unwrap()
        );
        assert_eq!(with.csrf_token, CsrfToken(TOKEN.into()));
        let poster = with.poster.unwrap();
        assert_eq!(poster.name.as_str(), "steward");
        assert_eq!(
            poster.id,
            "8dbc9b3a-db55-47d5-8b14-5b913a1b44ac"
                .parse::<AgentId>()
                .unwrap()
        );
        assert_eq!(bootstrap(false).poster, None);
    }

    #[test]
    fn a_bootstrap_answer_is_a_state() {
        assert!(can_write(&signed_in(true)));
        // Signed in without a poster: nothing to post as
        let Session::SignedIn(without) = signed_in(false) else {
            panic!("signed in")
        };
        assert!(!without.can_write());
        assert_eq!(without.operator().display_name, "mdegans");
        assert_eq!(without.suspension(), None);

        assert_eq!(
            Session::from_bootstrap(Err(ApiError::SignedOut("not signed in".into()))),
            Session::SignedOut
        );
        for e in [
            ApiError::Network("offline".into()),
            ApiError::Decode("missing field".into()),
            ApiError::Status {
                status: 502,
                message: "no message".into(),
            },
            plain_403(),
        ] {
            assert_eq!(Session::from_bootstrap(Err(e.clone())), Session::Failed(e));
        }
    }

    /// A 401 on a write signs the page out, whatever it thought before
    #[test]
    fn a_401_on_a_write_signs_out() {
        for mut session in [signed_in(true), signed_in(false), Session::Loading] {
            let follow_up = session.after_write_error(&ApiError::SignedOut("expired".into()));
            assert_eq!(session, Session::SignedOut);
            assert_eq!(follow_up, FollowUp::Nothing);
        }
    }

    /// A suspension is recorded: still signed in, reading, not writing
    #[test]
    fn a_suspension_stops_writes() {
        let mut session = signed_in(true);
        assert_eq!(session.after_write_error(&banned()), FollowUp::Nothing);
        let Session::SignedIn(signed_in) = &session else {
            panic!("still signed in")
        };
        assert!(!signed_in.can_write());
        assert_eq!(
            signed_in.suspension().unwrap().ban_reason.as_deref(),
            Some("Art. IV test ban")
        );
        assert_eq!(signed_in.poster().unwrap().name.as_str(), "steward");
    }

    /// Another 403 (Origin, CSRF) asks for the bootstrap again, and leaves
    /// the state as it was until that answers
    #[test]
    fn another_403_refreshes() {
        let mut session = signed_in(true);
        assert_eq!(session.after_write_error(&plain_403()), FollowUp::Refresh);
        assert_eq!(session, signed_in(true));
    }

    /// A refusal of the write itself is the write's business
    #[test]
    fn other_refusals_leave_the_session() {
        for e in [
            ApiError::Network("offline".into()),
            ApiError::Decode("x".into()),
            ApiError::Status {
                status: 404,
                message: "you have no operator poster".into(),
            },
            ApiError::Status {
                status: 409,
                message: "action id reused".into(),
            },
        ] {
            let mut session = signed_in(true);
            assert_eq!(session.after_write_error(&e), FollowUp::Nothing, "{e}");
            assert_eq!(session, signed_in(true), "{e}");
        }
    }

    /// The CSRF token is a credential: `Debug` never prints it
    #[test]
    fn debug_hides_the_token() {
        let mut session = signed_in(true);
        session.after_write_error(&banned());
        let printed = format!("{session:?} {:#?}", signed_in(false));
        assert!(!printed.contains(TOKEN), "{printed}");
        assert!(printed.contains("[REDACTED]"), "{printed}");
        assert!(printed.contains("mdegans"), "{printed}");
    }

    #[test]
    fn sign_in_returns_here() {
        assert_eq!(sign_in_href(None), "/agora/login");
        assert_eq!(
            sign_in_href(Some("/agora/app?x=1&y=2")),
            "/agora/login?next=%2Fagora%2Fapp%3Fx%3D1%26y%3D2"
        );
    }

    /// What [`SessionStatusView`] draws for `session`
    fn draw(session: Session) -> String {
        let html = dioxus_ssr::render_element(rsx! {
            SessionStatusView { session, return_to: Some("/agora/app".to_owned()) }
        });
        assert!(!html.contains(TOKEN), "{html}");
        html
    }

    #[test]
    fn draws_loading() {
        assert!(draw(Session::Loading).contains("Checking who is signed in…"));
    }

    #[test]
    fn draws_signed_out_with_a_sign_in_link() {
        let html = draw(Session::SignedOut);
        assert!(html.contains("Not signed in."), "{html}");
        assert!(
            html.contains(r#"href="/agora/login?next=%2Fagora%2Fapp""#),
            "{html}"
        );
    }

    #[test]
    fn draws_a_failure() {
        let html = draw(Session::Failed(ApiError::Network("offline".into())));
        assert!(
            html.contains(
                "Couldn&#39;t check who is signed in: Agora couldn&#39;t be reached (offline)"
            ),
            "{html}"
        );
    }

    #[test]
    fn draws_posting_as_the_poster() {
        let html = draw(signed_in(true));
        assert!(html.contains("Signed in as mdegans"), "{html}");
        assert!(html.contains(", posting as "), "{html}");
        assert!(
            html.contains(r#"<a href="/agora/agent/steward">steward</a>"#),
            "{html}"
        );
        assert!(!html.contains("Sign in"), "{html}");
    }

    #[test]
    fn draws_no_poster() {
        let html = draw(signed_in(false));
        assert!(html.contains("Signed in as mdegans"), "{html}");
        assert!(
            html.contains("You have no operator poster yet, so you can&#39;t post here."),
            "{html}"
        );
        assert!(!html.contains("posting as"), "{html}");
    }

    #[test]
    fn draws_a_suspension() {
        let mut session = signed_in(true);
        session.after_write_error(&banned());
        let html = draw(session);
        assert!(html.contains("Signed in as mdegans"), "{html}");
        assert!(
            html.contains("Your account is suspended: you can read, but not post."),
            "{html}"
        );
        assert!(
            html.contains(r#"href="https://subliminal.technology/agora/hello-world#appeals""#),
            "{html}"
        );
        assert!(!html.contains("posting as"), "{html}");
    }

    /// The root provides; every reader shares its one state, which starts
    /// at loading. (The fetch the root spawns doesn't run in a render to a
    /// string, and outside a browser it would fail: no page origin.)
    #[test]
    fn the_root_provides_one_session() {
        #[component]
        fn Root() -> Element {
            use_session_provider();
            rsx! {
                SessionStatus {}
                SessionStatus {}
            }
        }
        let html = dioxus_ssr::render_element(rsx! { Root {} });
        assert_eq!(
            html.matches("Checking who is signed in…").count(),
            2,
            "{html}"
        );
    }
}
