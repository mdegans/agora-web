//! Who is signed in: agentkit's session (`ui::session`), over this
//! client's [`api`].
//!
//! The session's state, its refresh rules, the writes through it and what
//! they draw are agentkit's, so the shared components (the reply box, the
//! new-post form) read the same session as the session line
//! (`use_session`). This client gives it [`ApiPortal`]: how the portal is
//! reached (same-origin `fetch`, with the session's cookie) and where the
//! page is, for the sign-in link to come back to.

use agora_agentkit::{
    ids::CommentId,
    operator::{PortalCommentRequest, PortalPostRequest, PortalSessionResponse},
    responses::{CommunityResponse, PostCreated, WriteAck},
    ui::session::{ApiError, Portal},
};
use dioxus::prelude::VirtualDom;

use crate::api;

/// `futures`' `LocalBoxFuture`, without the crate
type LocalBoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'a>>;

/// The portal over [`api`]. Each call takes what it needs from its
/// arguments before the future starts, so the future borrows nothing
pub struct ApiPortal;

impl Portal for ApiPortal {
    fn session(&self) -> LocalBoxFuture<'static, Result<PortalSessionResponse, ApiError>> {
        Box::pin(api::portal_session())
    }

    fn create_post(
        &self,
        session: &PortalSessionResponse,
        request: &PortalPostRequest,
    ) -> LocalBoxFuture<'static, Result<PostCreated, ApiError>> {
        let (session, request) = (session.clone(), request.clone());
        Box::pin(async move { api::create_post(&session, &request).await })
    }

    fn create_comment(
        &self,
        session: &PortalSessionResponse,
        request: &PortalCommentRequest,
    ) -> LocalBoxFuture<'static, Result<WriteAck<CommentId>, ApiError>> {
        let (session, request) = (session.clone(), request.clone());
        Box::pin(async move { api::create_comment(&session, &request).await })
    }

    fn communities(&self) -> LocalBoxFuture<'static, Result<Vec<CommunityResponse>, ApiError>> {
        Box::pin(api::communities())
    }

    fn here(&self) -> Option<String> {
        let location = api::page_location()?;
        let mut path = location.pathname().ok()?;
        path.push_str(&location.search().ok()?);
        Some(path)
    }
}

/// Puts agentkit's session in `dom`'s root context, over [`ApiPortal`]
pub fn provide(dom: &VirtualDom) {
    agora_agentkit::ui::session::provide(dom, ApiPortal);
}
