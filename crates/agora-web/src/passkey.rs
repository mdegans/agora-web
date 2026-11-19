//! The browser's passkey prompt, for the tap a portal message takes
//! (agentkit's `Portal::passkey`).
//!
//! **The prompt opens before [`get`] returns.** A browser shows a WebAuthn
//! prompt only inside a user gesture, and Safari refuses one when an
//! awaited request came between the press and the call (the PRF test page,
//! agora `static/prf-test.js`, calls `navigator.credentials.get` straight
//! from its click handler for the same reason). agentkit calls this from
//! the press itself ("Tap your passkey to send"), after the tap's options
//! were already fetched by the press before ("Send"); the future only waits
//! for the answer.
//!
//! The options are the server's `PublicKeyCredentialRequestOptionsJSON`
//! (binary fields base64url), read with
//! `PublicKeyCredential.parseRequestOptionsFromJSON` where the browser has
//! it and by hand otherwise; the answer is `PublicKeyCredential.toJSON()`,
//! or the same shape by hand, as agora's `static/passkeys.js` does. Through
//! `js_sys::Reflect`, so no web-sys feature is needed for it.

use agora_agentkit::ui::session::TapFailed;

/// `futures`' `LocalBoxFuture`, without the crate
type LocalBoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'a>>;

/// The `publicKey` part of the server's options: the options themselves,
/// or their `publicKey` when they come wrapped (`{"publicKey": …}`)
pub fn public_key(options: &serde_json::Value) -> &serde_json::Value {
    options.get("publicKey").unwrap_or(options)
}

/// Opens the browser's passkey prompt for `options` now, and answers with
/// what the person tapped (`PublicKeyCredential.toJSON()`). Off the browser
/// (the host tests) there is no prompt
pub fn get(
    options: &serde_json::Value,
) -> LocalBoxFuture<'static, Result<serde_json::Value, TapFailed>> {
    #[cfg(target_arch = "wasm32")]
    return wasm::get(public_key(options));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = public_key(options);
        Box::pin(std::future::ready(Err(TapFailed::Unsupported(
            "not in a browser".into(),
        ))))
    }
}

/// base64url (no padding) to bytes; `None` for text that isn't
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
pub fn from_b64url(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in text.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Bytes to base64url, no padding
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
pub fn to_b64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::{
        cell::RefCell,
        future::Future,
        pin::Pin,
        rc::Rc,
        task::{Context, Poll, Waker},
    };

    use agora_agentkit::ui::session::TapFailed;
    use web_sys::{
        js_sys::{self, Array, Function, JSON, Object, Promise, Reflect, Uint8Array},
        wasm_bindgen::{JsCast, JsValue, closure::Closure},
    };

    use super::{LocalBoxFuture, from_b64url, to_b64url};

    /// See [`super::get`]: the prompt is opened here, synchronously
    pub fn get(
        options: &serde_json::Value,
    ) -> LocalBoxFuture<'static, Result<serde_json::Value, TapFailed>> {
        let started = start(options);
        Box::pin(async move {
            let credential = Settled::new(started?).await.map_err(|e| failure(&e))?;
            if credential.is_null() || credential.is_undefined() {
                return Err(TapFailed::Failed("no passkey answer came back".into()));
            }
            to_json(&credential)
        })
    }

    fn field(target: &JsValue, name: &str) -> JsValue {
        Reflect::get(target, &JsValue::from_str(name)).unwrap_or(JsValue::UNDEFINED)
    }

    fn set(target: &JsValue, name: &str, value: &JsValue) -> Result<(), TapFailed> {
        Reflect::set(target, &JsValue::from_str(name), value)
            .map(|_| ())
            .map_err(|e| TapFailed::Failed(described(&e)))
    }

    /// `navigator.credentials.get({publicKey})`, called now
    fn start(options: &serde_json::Value) -> Result<Promise, TapFailed> {
        let window = web_sys::window().ok_or_else(|| TapFailed::Unsupported("no window".into()))?;
        let credentials = field(&field(&window, "navigator"), "credentials");
        let get = field(&credentials, "get");
        let get: Function = get
            .dyn_into()
            .map_err(|_| TapFailed::Unsupported("no passkeys (WebAuthn) in this browser".into()))?;
        let public_key = request_options(&window, options)?;
        let argument = Object::new();
        set(&argument, "publicKey", &public_key)?;
        let promise = get
            .call1(&credentials, &argument)
            .map_err(|e| failure(&e))?;
        promise
            .dyn_into::<Promise>()
            .map_err(|_| TapFailed::Failed("credentials.get answered no promise".into()))
    }

    /// The options as the browser takes them: through
    /// `PublicKeyCredential.parseRequestOptionsFromJSON` where it exists,
    /// else with the binary fields (the challenge, each credential's id)
    /// decoded from base64url here
    fn request_options(
        window: &JsValue,
        options: &serde_json::Value,
    ) -> Result<JsValue, TapFailed> {
        let text = serde_json::to_string(options).map_err(|e| TapFailed::Failed(e.to_string()))?;
        let parsed = JSON::parse(&text).map_err(|e| TapFailed::Failed(described(&e)))?;
        let class = field(window, "PublicKeyCredential");
        if class.is_undefined() {
            return Err(TapFailed::Unsupported(
                "no passkeys (WebAuthn) in this browser".into(),
            ));
        }
        if let Ok(parse) = field(&class, "parseRequestOptionsFromJSON").dyn_into::<Function>() {
            return parse.call1(&class, &parsed).map_err(|e| failure(&e));
        }
        let challenge = options
            .get("challenge")
            .and_then(|c| c.as_str())
            .and_then(from_b64url)
            .ok_or_else(|| TapFailed::Failed("the tap's options have no challenge".into()))?;
        set(&parsed, "challenge", &bytes(&challenge))?;
        let allowed = field(&parsed, "allowCredentials");
        if let Ok(allowed) = allowed.dyn_into::<Array>() {
            for credential in allowed.iter() {
                let id = field(&credential, "id")
                    .as_string()
                    .and_then(|id| from_b64url(&id))
                    .ok_or_else(|| TapFailed::Failed("a credential id isn't base64url".into()))?;
                set(&credential, "id", &bytes(&id))?;
            }
        }
        Ok(parsed)
    }

    /// `bytes` as an `ArrayBuffer`
    fn bytes(bytes: &[u8]) -> JsValue {
        Uint8Array::from(bytes).buffer().into()
    }

    /// An `ArrayBuffer` field as base64url; `None` when absent
    fn b64_field(target: &JsValue, name: &str) -> Option<String> {
        let value = field(target, name);
        if value.is_null() || value.is_undefined() {
            return None;
        }
        Some(to_b64url(&Uint8Array::new(&value).to_vec()))
    }

    /// `credential.toJSON()` where the browser has it, else the same shape
    /// by hand
    fn to_json(credential: &JsValue) -> Result<serde_json::Value, TapFailed> {
        let json = match field(credential, "toJSON").dyn_into::<Function>() {
            Ok(to_json) => to_json.call0(credential).map_err(|e| failure(&e))?,
            Err(_) => {
                let response = field(credential, "response");
                let out = Object::new();
                set(&out, "id", &field(credential, "id"))?;
                set(&out, "type", &field(credential, "type"))?;
                let raw = b64_field(credential, "rawId").unwrap_or_default();
                set(&out, "rawId", &JsValue::from_str(&raw))?;
                set(
                    &out,
                    "authenticatorAttachment",
                    &field(credential, "authenticatorAttachment"),
                )?;
                let extensions = field(credential, "getClientExtensionResults")
                    .dyn_into::<Function>()
                    .ok()
                    .and_then(|f| f.call0(credential).ok())
                    .unwrap_or_else(|| Object::new().into());
                set(&out, "clientExtensionResults", &extensions)?;
                let inner = Object::new();
                for name in [
                    "clientDataJSON",
                    "authenticatorData",
                    "signature",
                    "userHandle",
                ] {
                    if let Some(text) = b64_field(&response, name) {
                        set(&inner, name, &JsValue::from_str(&text))?;
                    }
                }
                set(&out, "response", &inner)?;
                out.into()
            }
        };
        let text = JSON::stringify(&json)
            .map_err(|e| TapFailed::Failed(described(&e)))?
            .as_string()
            .unwrap_or_default();
        serde_json::from_str(&text).map_err(|e| TapFailed::Failed(e.to_string()))
    }

    /// A DOMException's `name: message`, or whatever it says of itself
    fn described(e: &JsValue) -> String {
        let name = field(e, "name").as_string();
        let message = field(e, "message").as_string();
        match (name, message) {
            (Some(name), Some(message)) => format!("{name}: {message}"),
            (Some(name), None) => name,
            (None, Some(message)) => message,
            (None, None) => js_sys::JsString::from(e.clone()).into(),
        }
    }

    /// What a refused prompt means: WebAuthn's `NotAllowedError` is a
    /// cancel or a timeout (the two aren't told apart, by design)
    fn failure(e: &JsValue) -> TapFailed {
        match field(e, "name").as_string().as_deref() {
            Some("NotAllowedError" | "AbortError") => TapFailed::Cancelled,
            Some("NotSupportedError" | "SecurityError") => TapFailed::Unsupported(described(e)),
            _ => TapFailed::Failed(described(e)),
        }
    }

    /// A promise as a future, without wasm-bindgen-futures: `then` stores
    /// the outcome and wakes the task
    struct Settled {
        state: Rc<RefCell<State>>,
        /// Kept alive until the promise settles: resolved, rejected
        _handlers: [Handler; 2],
    }

    /// A promise's `then` callback
    type Handler = Closure<dyn FnMut(JsValue)>;

    #[derive(Default)]
    struct State {
        outcome: Option<Result<JsValue, JsValue>>,
        waker: Option<Waker>,
    }

    impl Settled {
        fn new(promise: Promise) -> Self {
            let state = Rc::new(RefCell::new(State::default()));
            let settle = |ok: bool| {
                let state = state.clone();
                Closure::<dyn FnMut(JsValue)>::new(move |value: JsValue| {
                    let mut state = state.borrow_mut();
                    state.outcome = Some(if ok { Ok(value) } else { Err(value) });
                    if let Some(waker) = state.waker.take() {
                        waker.wake();
                    }
                })
            };
            let (resolved, rejected) = (settle(true), settle(false));
            // The promise `then2` makes is dropped: both outcomes are handled
            let _ = promise.then2(&resolved, &rejected);
            Self {
                state,
                _handlers: [resolved, rejected],
            }
        }
    }

    impl Future for Settled {
        type Output = Result<JsValue, JsValue>;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let mut state = self.state.borrow_mut();
            match state.outcome.take() {
                Some(outcome) => Poll::Ready(outcome),
                None => {
                    state.waker = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// base64url both ways, as WebAuthn's JSON writes it (no padding)
    #[test]
    fn base64url_round_trips() {
        for bytes in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
            &[0xfb, 0xff, 0xfe],
        ] {
            let text = to_b64url(bytes);
            assert!(!text.contains('='), "{text}");
            assert_eq!(from_b64url(&text).as_deref(), Some(bytes), "{text}");
        }
        assert_eq!(to_b64url(b"foobar"), "Zm9vYmFy");
        assert_eq!(to_b64url(&[0xfb, 0xff, 0xfe]), "-__-");
        assert_eq!(from_b64url("not base64!"), None);
    }

    /// Wrapped or not, the `publicKey` part is what the prompt takes
    #[test]
    fn the_public_key_part_is_read_either_way() {
        let inner: serde_json::Value =
            serde_json::from_str(r#"{"challenge":"AAAA","rpId":"localhost"}"#).unwrap();
        assert_eq!(public_key(&inner), &inner);
        let wrapped: serde_json::Value =
            serde_json::from_str(r#"{"publicKey":{"challenge":"AAAA","rpId":"localhost"}}"#)
                .unwrap();
        assert_eq!(public_key(&wrapped), &inner);
    }
}
