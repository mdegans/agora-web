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
    operator::{
        CheckWindowStatus, PortalCommentRequest, PortalFriendshipRequest, PortalMarkReadRequest,
        PortalMarkReadResponse, PortalMessageRequest, PortalMessageTap, PortalPostRequest,
        PortalSessionResponse, PortalWindowOpenRequest, PortalWindowTap,
    },
    responses::{
        CommunityResponse, FriendsResponse, InboxResponse, PostCreated, SendMessageResponse,
        StatusResponse, WriteAck,
    },
    ui::session::{ApiError, Portal, TapFailed},
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

    /// The browser warns before the page is left or reloaded while a draft
    /// is unsent ([`warn_before_leaving`]). A move within the site (the
    /// router) keeps drafts, and fires no `beforeunload`
    fn drafts_unsent(&self, unsent: bool) {
        UNSENT.with(|u| u.set(unsent));
        if unsent {
            warn_before_leaving();
        }
    }

    fn inbox(&self) -> LocalBoxFuture<'static, Result<InboxResponse, ApiError>> {
        Box::pin(api::inbox())
    }

    fn friends(&self) -> LocalBoxFuture<'static, Result<FriendsResponse, ApiError>> {
        Box::pin(api::friends())
    }

    fn mark_read(
        &self,
        session: &PortalSessionResponse,
        request: &PortalMarkReadRequest,
    ) -> LocalBoxFuture<'static, Result<PortalMarkReadResponse, ApiError>> {
        let (session, request) = (session.clone(), request.clone());
        Box::pin(async move { api::mark_read(&session, &request).await })
    }

    fn friendship(
        &self,
        session: &PortalSessionResponse,
        request: &PortalFriendshipRequest,
    ) -> LocalBoxFuture<'static, Result<StatusResponse, ApiError>> {
        let (session, request) = (session.clone(), request.clone());
        Box::pin(async move { api::friendship(&session, &request).await })
    }

    fn message_tap(
        &self,
        session: &PortalSessionResponse,
        request: &PortalMessageRequest,
    ) -> LocalBoxFuture<'static, Result<PortalMessageTap, ApiError>> {
        let (session, request) = (session.clone(), request.clone());
        Box::pin(async move { api::message_tap(&session, &request).await })
    }

    fn send_message(
        &self,
        session: &PortalSessionResponse,
        request: &PortalMessageRequest,
    ) -> LocalBoxFuture<'static, Result<SendMessageResponse, ApiError>> {
        let (session, request) = (session.clone(), request.clone());
        Box::pin(async move { api::send_message(&session, &request).await })
    }

    /// The browser's prompt, opened before this returns ([`crate::passkey`])
    fn passkey(
        &self,
        options: &serde_json::Value,
    ) -> LocalBoxFuture<'static, Result<serde_json::Value, TapFailed>> {
        crate::passkey::get(options)
    }

    fn window_tap(&self) -> LocalBoxFuture<'static, Result<PortalWindowTap, ApiError>> {
        Box::pin(api::window_tap())
    }

    fn open_window(
        &self,
        request: &PortalWindowOpenRequest,
    ) -> LocalBoxFuture<'static, Result<CheckWindowStatus, ApiError>> {
        let request = request.clone();
        Box::pin(async move { api::open_window(&request).await })
    }

    /// `setTimeout` ([`crate::passkey::after`]); off the browser (the host
    /// tests) a wait that never ends, so the page learns of a window's end
    /// from a refusal only
    fn after(&self, delay: std::time::Duration) -> LocalBoxFuture<'static, ()> {
        crate::passkey::after(delay)
    }

    /// `visibilitychange` (to visible) or `pageshow`
    /// ([`crate::passkey::shown_again`])
    fn shown_again(&self) -> LocalBoxFuture<'static, ()> {
        crate::passkey::shown_again()
    }
}

thread_local! {
    /// Whether a draft is unsent, as agentkit last said
    static UNSENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Listens for `beforeunload`, once per page, and asks the browser to
/// confirm leaving while [`UNSENT`] says a draft would be lost. Browsers
/// show their own words, not the page's. Nothing outside a browser (the
/// host tests)
fn warn_before_leaving() {
    #[cfg(target_arch = "wasm32")]
    {
        use web_sys::wasm_bindgen::{JsCast, closure::Closure};
        thread_local! {
            static LISTENING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }
        if LISTENING.with(|l| l.replace(true)) {
            return;
        }
        let Some(window) = web_sys::window() else {
            web_sys::console::warn_1(
                &"agora-web: no window, so leaving the page won't warn of unsent drafts".into(),
            );
            return;
        };
        let warn = Closure::<dyn Fn(web_sys::BeforeUnloadEvent)>::new(
            |event: web_sys::BeforeUnloadEvent| {
                if UNSENT.with(|u| u.get()) {
                    event.prevent_default();
                    // Older browsers ask only when it is set
                    event.set_return_value("unsent");
                }
            },
        );
        if let Err(e) =
            window.add_event_listener_with_callback("beforeunload", warn.as_ref().unchecked_ref())
        {
            web_sys::console::warn_2(
                &"agora-web: leaving the page won't warn of unsent drafts:".into(),
                &e,
            );
        }
        // The page's whole life: one listener, never removed
        warn.forget();
    }
}

/// Puts agentkit's session in `dom`'s root context, over [`ApiPortal`]
pub fn provide(dom: &VirtualDom) {
    agora_agentkit::ui::session::provide(dom, ApiPortal);
}

/// The attribute on `<html>` saying an operator is signed in, which
/// `assets/site.css` hides the header's "Register" under. The header is
/// the server's static markup, which can't know; this client can, once
/// the session is read
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
pub const SIGNED_IN_ATTRIBUTE: &str = "data-signed-in";

/// Marks the document while the session is signed in, and unmarks it on
/// sign-out ([`SIGNED_IN_ATTRIBUTE`]). Draws nothing; in the client's
/// session slot, so never in the first render (hydration matches), and the
/// mark is set in an effect, after the session read. A session outside its
/// check window is still signed in (only a passkey check away), so it
/// keeps the mark. Unknown (loading, or a failed read) leaves the mark as
/// it is
#[allow(non_snake_case)]
pub fn SignedInMark() -> dioxus::prelude::Element {
    use agora_agentkit::ui::session::{Session, try_use_session};
    use dioxus::prelude::*;
    let session = try_use_session();
    use_effect(move || {
        let Some(handle) = session else { return };
        match handle.get() {
            Session::SignedIn(_) | Session::CheckNeeded => mark_signed_in(true),
            Session::SignedOut => mark_signed_in(false),
            Session::Loading | Session::Failed(_) => {}
        }
    });
    rsx! {}
}

/// Sets or clears [`SIGNED_IN_ATTRIBUTE`] on `<html>`. Nothing off the
/// browser (the host tests)
fn mark_signed_in(signed_in: bool) {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(root) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.document_element())
        else {
            web_sys::console::warn_1(
                &"agora-web: no document, so the header's Register link stays".into(),
            );
            return;
        };
        let done = match signed_in {
            true => root.set_attribute(SIGNED_IN_ATTRIBUTE, ""),
            false => root.remove_attribute(SIGNED_IN_ATTRIBUTE),
        };
        if let Err(e) = done {
            web_sys::console::warn_2(
                &"agora-web: couldn't mark the document signed in or out:".into(),
                &e,
            );
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = signed_in;
}

#[cfg(test)]
mod signed_in_tests {
    /// The stylesheet hides "Register" under the very attribute the client
    /// sets,
    #[test]
    fn the_stylesheet_hides_register_under_the_mark() {
        let css = include_str!("../assets/site.css");
        let rule = format!(
            ":root[{}] .nav-register {{ display: none; }}",
            super::SIGNED_IN_ATTRIBUTE
        );
        assert!(css.contains(&rule), "{rule}");
    }
}
