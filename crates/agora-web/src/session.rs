//! Who is signed in: the portal session, as the UI reads it.
//!
//! **One fetch per page.** The island's root ([`crate::App`]) calls
//! [`use_session_provider`] once; it puts a [`SessionHandle`] in Dioxus
//! context and fetches the bootstrap (`GET /agora/api/portal/session`) when
//! the root mounts. Every other component reads it with [`use_session`] and
//! never fetches it itself. It is fetched again only when asked
//! ([`SessionHandle::refresh`], the "Try again" after a failure), or when a
//! write's refusal says the session may have changed. Should a page ever
//! host two islands, each root fetches its own.
//!
//! **A refresh never takes a signed-in page away for a hiccup.** Only a 200
//! or a 401 replaces a [`Session::SignedIn`]; a refresh that fails otherwise
//! (offline, a 5xx) leaves it as it was ([`Session::after_refresh`]).
//!
//! **Writes go through the handle** ([`SessionHandle::create_post`],
//! [`SessionHandle::create_comment`]): they take the CSRF token from the
//! [`SignedIn`] value the component drew, and a refusal updates the state
//! when it was this session's ([`Session::after_write_error`]): a 401 signs
//! the page out, a suspension is recorded and [`SignedIn::can_write`] turns
//! false; any other 403 (a stale CSRF token, say after signing in again in
//! another tab) and a 404 (the server's answer for "no operator poster" as
//! well as for an unknown community or `reply_to`) fetch the bootstrap
//! again.
//!
//! **For a compose form (step 4c):** a 401 turns the state into
//! [`Session::SignedOut`], so a form drawn only `if let Session::SignedIn`
//! unmounts, and its draft with it. If a draft is to survive signing in
//! again, keep it outside that conditional: in a signal owned by an
//! ancestor that is always mounted. And spawn the write itself where it
//! outlives the form (`dioxus::core::spawn_forever`, which runs on the root
//! scope): reqwest's wasm client aborts the `fetch` when its future is
//! dropped, and a task spawned by a component is dropped when the component
//! unmounts, so a form that closes mid-POST would abandon a write the
//! server may already have committed. See the README.
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
    responses::{BanInfoResponse, PostCreated, WriteAck},
};
use dioxus::prelude::*;
use url::{Position, Url};

use crate::api::{self, ApiError, BanInfoExt};

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
    /// No one is signed in (the bootstrap, or a write, was a 401). The way
    /// back is the sign-in link, which returns to this page and so loads it
    /// again
    SignedOut,
    /// An operator is signed in
    SignedIn(SignedIn),
    /// The bootstrap failed for another reason (offline, a 5xx, an answer
    /// this client can't read); whether anyone is signed in is unknown.
    /// The way back is [`SessionHandle::refresh`] (the view's "Try again")
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

    /// Whether `other` is the same session: the same operator and the same
    /// CSRF token (one per session; compared in constant time)
    fn is_session_of(&self, other: &SignedIn) -> bool {
        self.portal.operator.id == other.portal.operator.id
            && self.portal.csrf_token == other.portal.csrf_token
    }
}

/// What a caller should do after [`Session::after_write_error`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowUp {
    /// Nothing: the state says what the refusal meant
    Nothing,
    /// Fetch the bootstrap again: the session may have changed under the
    /// page
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

    /// A refreshed bootstrap's answer, applied. A 200 or a 401 replaces the
    /// state. Any other failure replaces it only when no one is known to be
    /// signed in: a signed-in page (and any form drawn for it) stays while
    /// the server is unreachable. A 200 for the same session keeps a
    /// suspension the page has learned (the bootstrap doesn't carry one),
    /// so a lifted suspension shows after a reload.
    pub fn after_refresh(&mut self, answer: Result<PortalSessionResponse, ApiError>) {
        let mut next = Self::from_bootstrap(answer);
        match (&*self, &mut next) {
            (Self::SignedIn(_), Self::Failed(_)) => return,
            (Self::SignedIn(now), Self::SignedIn(new)) if now.is_session_of(new) => {
                new.suspension = now.suspension.clone();
            }
            _ => {}
        }
        *self = next;
    }

    /// What a write's refusal says about the session. `used` is the session
    /// the write was made with; the state is changed only if it is still
    /// that one, since a refusal of an older session's write says nothing
    /// about a newer one.
    ///
    /// - 401: signed out (or, if the session has changed since, ask again);
    /// - 403 for a suspension: recorded;
    /// - another 403 (Origin, a stale CSRF token): ask again;
    /// - 404: ask again. The server answers 404 when the operator has no
    ///   operator poster, and also for an unknown community or `reply_to`;
    ///   one GET tells which, and corrects a "posting as" that went stale;
    /// - anything else (a 400, 409, 5xx, a network error) is about the
    ///   write, not the session.
    pub fn after_write_error(&mut self, used: &SignedIn, error: &ApiError) -> FollowUp {
        let current = matches!(self, Self::SignedIn(now) if now.is_session_of(used));
        match error {
            ApiError::SignedOut(_) if current => {
                *self = Self::SignedOut;
                FollowUp::Nothing
            }
            ApiError::SignedOut(_) => FollowUp::Refresh,
            ApiError::Forbidden(_) => match error.suspension() {
                Some(ban) => {
                    if let Self::SignedIn(now) = self
                        && current
                    {
                        now.suspension = Some(Box::new(ban.clone()));
                    }
                    FollowUp::Nothing
                }
                None => FollowUp::Refresh,
            },
            ApiError::Status { status: 404, .. } => FollowUp::Refresh,
            ApiError::Network(_) | ApiError::Status { .. } | ApiError::Decode(_) => {
                FollowUp::Nothing
            }
        }
    }
}

/// The session in context: read it, refresh it, write through it. `Copy`,
/// like the signals it holds.
///
/// The signals are owned by the root scope, as the refresh task that
/// writes them runs there (`spawn_forever`); owned by `App`, a scope below
/// it, every write from the task would log dioxus's hoisting warning in a
/// debug build.
#[derive(Clone, Copy, PartialEq)]
pub struct SessionHandle {
    state: Signal<Session>,
    /// Bumped by every refresh, so a bootstrap that answers after a later
    /// refresh was started is dropped rather than overwriting it. Only
    /// refreshes compete on it: a write's refusal changes the state without
    /// it, so it can't cancel a refresh in flight.
    generation: Signal<u64>,
}

impl SessionHandle {
    /// A handle, its state loading, owned by the root scope
    fn new_root() -> Self {
        Self {
            state: Signal::new_in_scope(Session::Loading, ScopeId::ROOT),
            generation: Signal::new_in_scope(0, ScopeId::ROOT),
        }
    }

    /// The session, subscribing the calling component to its changes
    pub fn get(&self) -> Session {
        self.state.cloned()
    }

    /// Fetches the bootstrap again ([`Session::after_refresh`] says what
    /// its answer replaces). The current state stays until it answers. The
    /// fetch runs on the root scope, so a refresh a form starts isn't
    /// cancelled when the form goes away.
    pub fn refresh(self) {
        let generation = self.begin_refresh();
        dioxus::core::spawn_forever(async move {
            self.finish_refresh(generation, api::portal_session().await);
        });
    }

    /// A refresh's generation, now the current one
    fn begin_refresh(self) -> u64 {
        let mut generation = self.generation;
        let next = *generation.peek() + 1;
        generation.set(next);
        next
    }

    /// Applies a refresh's answer, unless a later refresh has started
    fn finish_refresh(self, generation: u64, answer: Result<PortalSessionResponse, ApiError>) {
        if *self.generation.peek() != generation {
            return;
        }
        let mut next = self.state.peek().clone();
        next.after_refresh(answer);
        self.replace(next);
    }

    /// Sets the state, if it changed (so readers don't redraw for nothing)
    fn replace(self, next: Session) {
        if next != *self.state.peek() {
            let mut state = self.state;
            state.set(next);
        }
    }
}

/// Writes, through the session. Nothing calls them yet: step 4c's compose
/// form is their first caller (and the `allow` goes with it).
#[allow(dead_code, reason = "for step 4c's compose form, its first caller")]
impl SessionHandle {
    /// Updates the state for the refusal of a write made with `used`, and
    /// refreshes if that's what it calls for
    pub fn note_write_error(self, used: &SignedIn, error: &ApiError) {
        let mut next = self.state.peek().clone();
        let follow_up = next.after_write_error(used, error);
        self.replace(next);
        if follow_up == FollowUp::Refresh {
            self.refresh();
        }
    }

    /// [`api::create_post`] with `signed_in`'s token; a refusal updates
    /// the session ([`Self::note_write_error`]). Spawn it where it outlives
    /// the form (see the module docs)
    pub async fn create_post(
        self,
        signed_in: &SignedIn,
        request: &PortalPostRequest,
    ) -> Result<PostCreated, ApiError> {
        let answer = api::create_post(signed_in.portal(), request).await;
        if let Err(e) = &answer {
            self.note_write_error(signed_in, e);
        }
        answer
    }

    /// [`api::create_comment`] with `signed_in`'s token; a refusal updates
    /// the session ([`Self::note_write_error`]). Spawn it where it outlives
    /// the form (see the module docs)
    pub async fn create_comment(
        self,
        signed_in: &SignedIn,
        request: &PortalCommentRequest,
    ) -> Result<WriteAck<CommentId>, ApiError> {
        let answer = api::create_comment(signed_in.portal(), request).await;
        if let Err(e) = &answer {
            self.note_write_error(signed_in, e);
        }
        answer
    }
}

/// Provides the session to everything below, and fetches the bootstrap
/// once, when the calling component mounts. Call it once, in the island's
/// root.
pub fn use_session_provider() -> SessionHandle {
    let handle = use_context_provider(SessionHandle::new_root);
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
/// [`use_session`], its "Try again" refreshing it
#[component]
pub fn SessionStatus() -> Element {
    let handle = use_session();
    let session = handle.get();
    rsx! {
        SessionStatusView {
            session,
            return_to: here(),
            on_retry: move |_| handle.refresh(),
        }
    }
}

/// Who is signed in, in one line: a sign-in link when no one is, and
/// whether the operator can post.
///
/// Every state is drawn in one `role="status"` paragraph, so a screen
/// reader announces the line as it changes (loading, then who is signed
/// in), politely; none of it is urgent enough for `alert`.
#[component]
pub fn SessionStatusView(
    /// What to draw
    session: Session,
    /// The page to come back to after signing in
    return_to: Option<String>,
    /// "Try again", after a failure
    on_retry: EventHandler,
) -> Element {
    match session {
        Session::Loading => rsx! {
            p { class: "post-meta", role: "status", "Checking who is signed in…" }
        },
        Session::SignedOut => {
            let href = sign_in_href(return_to.as_deref());
            rsx! {
                p { class: "post-meta", role: "status",
                    "Not signed in. "
                    a { href: "{href}", "Sign in" }
                    " to post as an operator."
                }
            }
        }
        Session::Failed(e) => rsx! {
            p { class: "post-meta", role: "status",
                "Couldn't check who is signed in: {e} "
                button {
                    class: "pill pill-btn",
                    r#type: "button",
                    onclick: move |_| on_retry.call(()),
                    "Try again"
                }
            }
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
        p { class: "post-meta", role: "status",
            "Signed in as {operator}"
            match (signed_in.suspension(), poster) {
                (Some(ban), _) => rsx! {
                    ". {ban.human_summary()}. "
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

    /// The bootstrap, as a typed value: agora-server builds this agentkit
    /// struct and serde writes it. Its bytes are tested once, in
    /// [`the_bootstrap_decodes`].
    fn bootstrap_with(poster: bool, token: &str) -> PortalSessionResponse {
        PortalSessionResponse {
            operator: OperatorRef {
                id: "34fb8f2e-0710-43c0-b592-a396969f1667".parse().unwrap(),
                display_name: "mdegans".into(),
            },
            csrf_token: CsrfToken(token.into()),
            poster: poster.then(|| AgentRef {
                id: "8dbc9b3a-db55-47d5-8b14-5b913a1b44ac".parse().unwrap(),
                name: "steward".parse().unwrap(),
            }),
        }
    }

    fn bootstrap(poster: bool) -> PortalSessionResponse {
        bootstrap_with(poster, TOKEN)
    }

    fn signed_in(poster: bool) -> Session {
        Session::from_bootstrap(Ok(bootstrap(poster)))
    }

    /// The `SignedIn` a write was made with
    fn used(session: &Session) -> SignedIn {
        match session {
            Session::SignedIn(s) => s.clone(),
            other => panic!("not signed in: {other:?}"),
        }
    }

    fn banned() -> ApiError {
        crate::api::tests::ban().into()
    }

    fn plain_403() -> ApiError {
        ApiError::Forbidden(ApiErrorBody::Plain(ErrorResponse {
            error: "a write must carry this session's CSRF token".into(),
        }))
    }

    /// A `Decode`, as reading a body this client can't makes one
    fn decode_error() -> ApiError {
        serde_json::from_str::<PortalSessionResponse>("{}")
            .unwrap_err()
            .into()
    }

    /// Whether `session` is signed in and may try a write
    fn can_write(session: &Session) -> bool {
        matches!(session, Session::SignedIn(s) if s.can_write())
    }

    /// The bootstrap's bytes, as agora-server sends them: the shape its
    /// `portal_api::tests::the_bootstrap_names_the_operator_its_token_and_its_poster`
    /// asserts and agentkit's `portal_session_wire` round-trips (agora has
    /// no on-disk wire snapshot of it: `snapshots/` holds the OpenAPI spec
    /// and the MCP tools). With a poster and without one, `poster` absent.
    #[test]
    fn the_bootstrap_decodes() {
        let with: PortalSessionResponse = serde_json::from_str(
            r#"{
                "operator": {
                    "id": "34fb8f2e-0710-43c0-b592-a396969f1667",
                    "display_name": "mdegans"
                },
                "csrf_token": "csrf-0f1e2d3c4b5a",
                "poster": {"id": "8dbc9b3a-db55-47d5-8b14-5b913a1b44ac", "name": "steward"}
            }"#,
        )
        .unwrap();
        assert_eq!(with, bootstrap(true));
        assert_eq!(
            with.operator.id,
            "34fb8f2e-0710-43c0-b592-a396969f1667"
                .parse::<OperatorId>()
                .unwrap()
        );
        assert_eq!(
            with.poster.unwrap().id,
            "8dbc9b3a-db55-47d5-8b14-5b913a1b44ac"
                .parse::<AgentId>()
                .unwrap()
        );
        let without: PortalSessionResponse = serde_json::from_str(
            r#"{
                "operator": {
                    "id": "34fb8f2e-0710-43c0-b592-a396969f1667",
                    "display_name": "mdegans"
                },
                "csrf_token": "csrf-0f1e2d3c4b5a"
            }"#,
        )
        .unwrap();
        assert_eq!(without, bootstrap(false));
    }

    #[test]
    fn a_bootstrap_answer_is_a_state() {
        assert!(can_write(&signed_in(true)));
        // Signed in without a poster: nothing to post as
        let without = used(&signed_in(false));
        assert!(!without.can_write());
        assert_eq!(without.operator().display_name, "mdegans");
        assert_eq!(without.suspension(), None);

        assert_eq!(
            Session::from_bootstrap(Err(ApiError::SignedOut("not signed in".into()))),
            Session::SignedOut
        );
        for e in [
            ApiError::Network("offline".into()),
            decode_error(),
            ApiError::Status {
                status: 502,
                message: "no message".into(),
            },
            plain_403(),
        ] {
            assert_eq!(Session::from_bootstrap(Err(e.clone())), Session::Failed(e));
        }
    }

    /// A refresh that fails other than with a 401 doesn't take a signed-in
    /// page away; a 401 or a 200 replaces anything
    #[test]
    fn a_failed_refresh_keeps_signed_in() {
        let mut session = signed_in(true);
        session.after_refresh(Err(ApiError::Network("offline".into())));
        assert_eq!(session, signed_in(true));
        session.after_refresh(Err(ApiError::SignedOut("expired".into())));
        assert_eq!(session, Session::SignedOut);
        session.after_refresh(Ok(bootstrap(false)));
        assert_eq!(session, signed_in(false));
        // Not signed in: a failure is shown
        let mut session = Session::SignedOut;
        session.after_refresh(Err(ApiError::Network("offline".into())));
        assert_eq!(
            session,
            Session::Failed(ApiError::Network("offline".into()))
        );
    }

    /// A refresh of the same session keeps the suspension the page learned;
    /// a new session starts clean
    #[test]
    fn a_refresh_keeps_a_known_suspension() {
        let mut session = signed_in(true);
        let write = used(&session);
        session.after_write_error(&write, &banned());
        session.after_refresh(Ok(bootstrap(true)));
        assert!(used(&session).suspension().is_some());
        session.after_refresh(Ok(bootstrap_with(true, "another-session")));
        assert!(used(&session).suspension().is_none());
    }

    /// A 401 on this session's write signs the page out
    #[test]
    fn a_401_on_a_write_signs_out() {
        for poster in [true, false] {
            let mut session = signed_in(poster);
            let write = used(&session);
            let follow_up =
                session.after_write_error(&write, &ApiError::SignedOut("expired".into()));
            assert_eq!(session, Session::SignedOut);
            assert_eq!(follow_up, FollowUp::Nothing);
        }
    }

    /// A refusal of an older session's write doesn't touch a newer one: a
    /// suspension isn't recorded, and a 401 asks the server instead
    #[test]
    fn an_older_sessions_refusal_leaves_the_new_one() {
        let old = used(&signed_in(true));
        let newer = Session::from_bootstrap(Ok(bootstrap_with(true, "newer-token")));

        let mut session = newer.clone();
        assert_eq!(
            session.after_write_error(&old, &banned()),
            FollowUp::Nothing
        );
        assert_eq!(session, newer);
        assert!(can_write(&session));

        let mut session = newer.clone();
        assert_eq!(
            session.after_write_error(&old, &ApiError::SignedOut("expired".into())),
            FollowUp::Refresh
        );
        assert_eq!(session, newer);
    }

    /// A suspension is recorded: still signed in, reading, not writing
    #[test]
    fn a_suspension_stops_writes() {
        let mut session = signed_in(true);
        let write = used(&session);
        assert_eq!(
            session.after_write_error(&write, &banned()),
            FollowUp::Nothing
        );
        let signed_in = used(&session);
        assert!(!signed_in.can_write());
        assert_eq!(
            signed_in.suspension().unwrap().ban_reason.as_deref(),
            Some("Art. IV test ban")
        );
        assert_eq!(signed_in.poster().unwrap().name.as_str(), "steward");
    }

    /// Another 403 (Origin, CSRF) and a 404 (perhaps no poster any more) ask
    /// for the bootstrap again, and leave the state until that answers
    #[test]
    fn another_403_or_a_404_refreshes() {
        for e in [
            plain_403(),
            ApiError::Status {
                status: 404,
                message: "you have no operator poster".into(),
            },
        ] {
            let mut session = signed_in(true);
            let write = used(&session);
            assert_eq!(
                session.after_write_error(&write, &e),
                FollowUp::Refresh,
                "{e}"
            );
            assert_eq!(session, signed_in(true), "{e}");
        }
    }

    /// A refusal of the write itself is the write's business
    #[test]
    fn other_refusals_leave_the_session() {
        for e in [
            ApiError::Network("offline".into()),
            decode_error(),
            ApiError::Status {
                status: 400,
                message: "unknown field `x`".into(),
            },
            ApiError::Status {
                status: 409,
                message: "this action id was already used for a different request; \
                          generate a new one for each action"
                    .into(),
            },
        ] {
            let mut session = signed_in(true);
            let write = used(&session);
            assert_eq!(
                session.after_write_error(&write, &e),
                FollowUp::Nothing,
                "{e}"
            );
            assert_eq!(session, signed_in(true), "{e}");
        }
    }

    /// The CSRF token is a credential: `Debug` never prints it
    #[test]
    fn debug_hides_the_token() {
        let mut session = signed_in(true);
        let write = used(&session);
        session.after_write_error(&write, &banned());
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

    /// What [`SessionStatusView`] draws for `session`: rendered from a
    /// component, since its `EventHandler` needs a runtime
    fn draw(session: Session) -> String {
        #[component]
        fn Drawn(session: Session) -> Element {
            rsx! {
                SessionStatusView {
                    session,
                    return_to: Some("/agora/app".to_owned()),
                    on_retry: |_| {},
                }
            }
        }
        let mut dom = VirtualDom::new_with_props(Drawn, DrawnProps { session });
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(!html.contains(TOKEN), "{html}");
        // One polite live region, whatever the state
        assert_eq!(html.matches(r#"role="status""#).count(), 1, "{html}");
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
    fn draws_a_failure_with_try_again() {
        let html = draw(Session::Failed(ApiError::Network("offline".into())));
        assert!(
            html.contains(
                "Couldn&#39;t check who is signed in: Agora couldn&#39;t be reached (offline)"
            ),
            "{html}"
        );
        assert!(
            html.contains(r#"<button class="pill pill-btn" type="button">Try again</button>"#),
            "{html}"
        );
        for other in [Session::Loading, Session::SignedOut, signed_in(true)] {
            assert!(!draw(other).contains("Try again"));
        }
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
        let write = used(&session);
        session.after_write_error(&write, &banned());
        let html = draw(session);
        assert!(html.contains("Signed in as mdegans"), "{html}");
        assert!(
            html.contains("Your account is suspended: you can read, but not post."),
            "{html}"
        );
        assert!(
            html.contains(r#"href="https://subliminal.technology/agora/protocol""#),
            "{html}"
        );
        assert!(!html.contains("posting as"), "{html}");
        assert!(!html.contains("file_appeal"), "{html}");
    }

    /// The root provides; every reader shares its one state, which starts
    /// at loading (the render to a string doesn't poll the fetch it spawns)
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

    // The handle, driven in a `VirtualDom` on the host. `process_events`
    // polls the tasks a refresh spawns; outside a browser the bootstrap
    // fetch fails at once (no page origin), so a real refresh settles in
    // one call.

    /// A mounted dom, and a handle made in it as the provider makes one
    fn dom_and_handle() -> (VirtualDom, SessionHandle) {
        fn empty() -> Element {
            rsx! {}
        }
        let mut dom = VirtualDom::new(empty);
        dom.rebuild_in_place();
        let handle = dom.in_scope(ScopeId::ROOT, SessionHandle::new_root);
        (dom, handle)
    }

    fn state(dom: &VirtualDom, handle: SessionHandle) -> Session {
        dom.in_scope(ScopeId::ROOT, || handle.state.peek().clone())
    }

    fn set(dom: &VirtualDom, handle: SessionHandle, session: Session) {
        dom.in_scope(ScopeId::ROOT, || handle.replace(session));
    }

    /// The real refresh: spawned on the root, its failure applied
    #[test]
    fn refresh_runs_and_settles() {
        let (mut dom, handle) = dom_and_handle();
        dom.in_scope(ScopeId::ROOT, || handle.refresh());
        assert_eq!(state(&dom, handle), Session::Loading);
        dom.process_events();
        assert_eq!(
            state(&dom, handle),
            Session::Failed(ApiError::Network("no window location".into()))
        );
    }

    /// The real refresh failing keeps a signed-in page signed in
    #[test]
    fn a_failing_refresh_keeps_signed_in() {
        let (mut dom, handle) = dom_and_handle();
        set(&dom, handle, signed_in(true));
        dom.in_scope(ScopeId::ROOT, || handle.refresh());
        dom.process_events();
        assert_eq!(state(&dom, handle), signed_in(true));
    }

    /// An answer to a refresh that a later one overtook is dropped,
    /// whichever order they answer in
    #[test]
    fn a_stale_refresh_answer_is_dropped() {
        let (dom, handle) = dom_and_handle();
        dom.in_scope(ScopeId::ROOT, || {
            let first = handle.begin_refresh();
            let second = handle.begin_refresh();
            handle.finish_refresh(second, Ok(bootstrap(true)));
            handle.finish_refresh(first, Err(ApiError::SignedOut("old".into())));
        });
        assert_eq!(state(&dom, handle), signed_in(true));
    }

    /// A write's refusal doesn't cancel a refresh in flight (only
    /// refreshes compete on the generation)
    #[test]
    fn a_write_refusal_doesnt_drop_a_refresh() {
        let (dom, handle) = dom_and_handle();
        set(&dom, handle, signed_in(true));
        let write = used(&signed_in(true));
        dom.in_scope(ScopeId::ROOT, || {
            let refresh = handle.begin_refresh();
            handle.note_write_error(&write, &banned());
            handle.finish_refresh(refresh, Ok(bootstrap_with(true, "newer-token")));
        });
        assert_eq!(
            state(&dom, handle),
            Session::from_bootstrap(Ok(bootstrap_with(true, "newer-token")))
        );
    }

    /// Through the handle, a suspension is recorded only on the session
    /// whose write it refused
    #[test]
    fn note_write_error_applies_to_the_matching_session_only() {
        let (dom, handle) = dom_and_handle();
        let newer = Session::from_bootstrap(Ok(bootstrap_with(true, "newer-token")));
        set(&dom, handle, newer.clone());
        let old = used(&signed_in(true));
        dom.in_scope(ScopeId::ROOT, || handle.note_write_error(&old, &banned()));
        assert_eq!(state(&dom, handle), newer);

        let current = used(&newer);
        dom.in_scope(ScopeId::ROOT, || {
            handle.note_write_error(&current, &banned())
        });
        assert!(used(&state(&dom, handle)).suspension().is_some());
    }

    /// A 404 on a write refreshes through the handle (here, outside a
    /// browser, the refresh fails, and the page stays signed in)
    #[test]
    fn a_404_refreshes_through_the_handle() {
        let (mut dom, handle) = dom_and_handle();
        set(&dom, handle, signed_in(true));
        let write = used(&signed_in(true));
        let generation = dom.in_scope(ScopeId::ROOT, || *handle.generation.peek());
        dom.in_scope(ScopeId::ROOT, || {
            handle.note_write_error(
                &write,
                &ApiError::Status {
                    status: 404,
                    message: "you have no operator poster".into(),
                },
            )
        });
        assert_eq!(
            dom.in_scope(ScopeId::ROOT, || *handle.generation.peek()),
            generation + 1
        );
        dom.process_events();
        assert_eq!(state(&dom, handle), signed_in(true));
    }
}
