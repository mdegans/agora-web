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

/// A future ready `delay` from now: the browser's `setTimeout`, awaited
/// as this module awaits the prompt's promise. agentkit's session waits
/// with it for a check window's end, to ask the server then. Off the
/// browser (the host tests), a wait that never ends
pub fn after(delay: std::time::Duration) -> LocalBoxFuture<'static, ()> {
    #[cfg(target_arch = "wasm32")]
    return wasm::after(delay);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = delay;
        Box::pin(std::future::pending())
    }
}

/// base64url (RFC 4648 §5, no padding, as WebAuthn's JSON writes it) to
/// bytes; `None` for text that isn't: a character outside the alphabet
/// (`+` and `/`, standard base64's, included), padding, a length no
/// encoding has (one past a multiple of four), or bits left over that an
/// encoder would have written as zero
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
pub fn from_b64url(text: &str) -> Option<Vec<u8>> {
    if text.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
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
    // What is left is padding bits, zero in a canonical encoding
    (acc == 0).then_some(out)
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

/// The binary members of the request options, decoded, for a browser
/// without `PublicKeyCredential.parseRequestOptionsFromJSON`: the
/// challenge, and each allowed credential's id in the options' order.
///
/// Only these are decoded. The options' `extensions` are passed on as
/// they came, so a binary member inside them (PRF's salts, say) would
/// reach the browser as text, not bytes; and only `publicKey` is passed to
/// `credentials.get`, so a `mediation` beside it is dropped. Both are fine
/// only while the message tap asks for no extensions and no mediation
/// (agora `security::start_message_tap`): one that does needs them decoded
/// here too, or the hand-built path refused for it.
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
#[derive(Debug, PartialEq)]
pub struct RequestBinary {
    pub challenge: Vec<u8>,
    pub allowed: Vec<Vec<u8>>,
}

#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
impl RequestBinary {
    /// Reads them from the options' `publicKey` part, or says which isn't
    /// base64url
    pub fn of(options: &serde_json::Value) -> Result<Self, TapFailed> {
        let challenge = options
            .get("challenge")
            .and_then(|c| c.as_str())
            .and_then(from_b64url)
            .ok_or_else(|| {
                TapFailed::Failed("the tap's options have no base64url challenge".into())
            })?;
        let allowed = match options.get("allowCredentials") {
            None | Some(serde_json::Value::Null) => Vec::new(),
            Some(serde_json::Value::Array(list)) => list
                .iter()
                .map(|c| {
                    c.get("id")
                        .and_then(|id| id.as_str())
                        .and_then(from_b64url)
                        .ok_or_else(|| TapFailed::Failed("a credential id isn't base64url".into()))
                })
                .collect::<Result<_, _>>()?,
            Some(_) => {
                return Err(TapFailed::Failed(
                    "the tap's allowCredentials isn't a list".into(),
                ));
            }
        };
        Ok(Self { challenge, allowed })
    }
}

/// What a browser without `PublicKeyCredential.toJSON()` answered, read
/// from its credential: built into the same JSON by [`Self::to_json`].
///
/// `extensions` is `getClientExtensionResults()` through `JSON.stringify`,
/// so a binary member in it is not encoded (an `ArrayBuffer` stringifies
/// to `{}`); fine only while the message tap asks for no extensions (see
/// [`RequestBinary`]).
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
#[derive(Debug, Default)]
pub struct Answer {
    pub id: String,
    pub kind: String,
    pub raw_id: Vec<u8>,
    pub attachment: Option<String>,
    pub extensions: serde_json::Map<String, serde_json::Value>,
    pub client_data_json: Vec<u8>,
    pub authenticator_data: Vec<u8>,
    pub signature: Vec<u8>,
    pub user_handle: Option<Vec<u8>>,
}

/// `PublicKeyCredential.toJSON()`'s shape for an assertion
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AnswerJson<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'a str,
    raw_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    authenticator_attachment: Option<&'a str>,
    client_extension_results: &'a serde_json::Map<String, serde_json::Value>,
    response: ResponseJson,
}

/// Its `response`: the binary members base64url
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ResponseJson {
    #[serde(rename = "clientDataJSON")]
    client_data_json: String,
    authenticator_data: String,
    signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_handle: Option<String>,
}

#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
impl Answer {
    /// `PublicKeyCredential.toJSON()`'s shape for an assertion: the binary
    /// members base64url, `userHandle` and `authenticatorAttachment` only
    /// when there is one
    pub fn to_json(&self) -> Result<serde_json::Value, TapFailed> {
        serde_json::to_value(AnswerJson {
            id: &self.id,
            kind: &self.kind,
            raw_id: to_b64url(&self.raw_id),
            authenticator_attachment: self.attachment.as_deref(),
            client_extension_results: &self.extensions,
            response: ResponseJson {
                client_data_json: to_b64url(&self.client_data_json),
                authenticator_data: to_b64url(&self.authenticator_data),
                signature: to_b64url(&self.signature),
                user_handle: self.user_handle.as_deref().map(to_b64url),
            },
        })
        .map_err(|e| TapFailed::Failed(e.to_string()))
    }
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

    use super::{Answer, LocalBoxFuture, RequestBinary};

    /// See [`super::after`]. `setTimeout` takes a signed 32-bit count of
    /// milliseconds; agentkit never asks for more than 20 days
    pub fn after(delay: std::time::Duration) -> LocalBoxFuture<'static, ()> {
        let ms = i32::try_from(delay.as_millis()).unwrap_or(i32::MAX);
        let promise = Promise::new(&mut |resolve: Function, _reject: Function| {
            let set = web_sys::window().map(|window| {
                window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            });
            match set {
                Some(Ok(_)) => {}
                Some(Err(e)) => web_sys::console::warn_2(
                    &"agora-web: no timer, so a check window's end is learned from a refusal only:"
                        .into(),
                    &e,
                ),
                None => web_sys::console::warn_1(
                    &"agora-web: no window, so a check window's end is learned from a refusal only"
                        .into(),
                ),
            }
        });
        Box::pin(async move {
            // A timer's promise only resolves
            let _ = Settled::new(promise).await;
        })
    }

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
        let binary = RequestBinary::of(options)?;
        set(&parsed, "challenge", &bytes(&binary.challenge))?;
        if let Ok(allowed) = field(&parsed, "allowCredentials").dyn_into::<Array>() {
            for (credential, id) in allowed.iter().zip(&binary.allowed) {
                set(&credential, "id", &bytes(id))?;
            }
        }
        Ok(parsed)
    }

    /// `bytes` as an `ArrayBuffer`
    fn bytes(bytes: &[u8]) -> JsValue {
        Uint8Array::from(bytes).buffer().into()
    }

    /// An `ArrayBuffer` field's bytes; `None` when absent
    fn bytes_field(target: &JsValue, name: &str) -> Option<Vec<u8>> {
        let value = field(target, name);
        if value.is_null() || value.is_undefined() {
            return None;
        }
        Some(Uint8Array::new(&value).to_vec())
    }

    /// A JS value as JSON, through `JSON.stringify`
    fn as_json(value: &JsValue) -> Result<serde_json::Value, TapFailed> {
        let text = JSON::stringify(value)
            .map_err(|e| TapFailed::Failed(described(&e)))?
            .as_string()
            .unwrap_or_default();
        serde_json::from_str(&text).map_err(|e| TapFailed::Failed(e.to_string()))
    }

    /// `credential.toJSON()` where the browser has it, else the same shape
    /// by hand ([`Answer`])
    fn to_json(credential: &JsValue) -> Result<serde_json::Value, TapFailed> {
        if let Ok(to_json) = field(credential, "toJSON").dyn_into::<Function>() {
            return as_json(&to_json.call0(credential).map_err(|e| failure(&e))?);
        }
        let response = field(credential, "response");
        let required = |name: &str| {
            bytes_field(&response, name)
                .ok_or_else(|| TapFailed::Failed(format!("the passkey's answer has no {name}")))
        };
        // No results, or results that aren't an object: none
        let extensions = match field(credential, "getClientExtensionResults").dyn_into::<Function>()
        {
            Ok(f) => match as_json(&f.call0(credential).map_err(|e| failure(&e))?)? {
                serde_json::Value::Object(results) => results,
                _ => serde_json::Map::new(),
            },
            Err(_) => serde_json::Map::new(),
        };
        Answer {
            id: field(credential, "id").as_string().unwrap_or_default(),
            kind: field(credential, "type").as_string().unwrap_or_default(),
            raw_id: bytes_field(credential, "rawId").unwrap_or_default(),
            attachment: field(credential, "authenticatorAttachment").as_string(),
            extensions,
            client_data_json: required("clientDataJSON")?,
            authenticator_data: required("authenticatorData")?,
            signature: required("signature")?,
            user_handle: bytes_field(&response, "userHandle"),
        }
        .to_json()
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

    /// Anything that isn't base64url as WebAuthn writes it is refused, not
    /// read as something else: standard base64's `+` and `/`, padding, a
    /// length no encoding has, and bits an encoder would have zeroed
    #[test]
    fn malformed_base64url_is_refused() {
        for text in [
            "+__-", "-__+", "/w", "_w/", "Zm9vYg==", "Zm9vYg=", "Zg=", "A", "Zm9vY",
            // "f" is "Zg"; "Zh" has the leftover bits set
            "Zh", " Zg", "Zg\n",
        ] {
            assert_eq!(from_b64url(text), None, "{text:?}");
        }
        assert_eq!(from_b64url("Zg").as_deref(), Some(&b"f"[..]));
        assert_eq!(from_b64url("").as_deref(), Some(&b""[..]));
    }

    fn json(text: &str) -> serde_json::Value {
        serde_json::from_str(text).unwrap()
    }

    /// The hand-built path decodes the challenge and each allowed
    /// credential's id, in order, and refuses options it can't read
    #[test]
    fn the_request_options_binary_members_are_decoded() {
        // "challenge", [1, 2, 3] and [0xfb, 0xff], base64url
        let options = json(
            r#"{"challenge": "Y2hhbGxlbmdl", "rpId": "localhost",
                "allowCredentials": [{"type": "public-key", "id": "AQID"},
                                     {"type": "public-key", "id": "-_8"}],
                "userVerification": "required"}"#,
        );
        assert_eq!(
            RequestBinary::of(&options).unwrap(),
            RequestBinary {
                challenge: b"challenge".to_vec(),
                allowed: vec![vec![1, 2, 3], vec![0xfb, 0xff]],
            }
        );
        let none = json(r#"{"challenge": "AAAA"}"#);
        assert_eq!(
            RequestBinary::of(&none).unwrap().allowed,
            Vec::<Vec<u8>>::new()
        );
        for bad in [
            r#"{"rpId": "localhost"}"#,
            r#"{"challenge": "AA+A"}"#,
            r#"{"challenge": 7}"#,
            r#"{"challenge": "AAAA", "allowCredentials": [{"id": "a/b"}]}"#,
            r#"{"challenge": "AAAA", "allowCredentials": [{"type": "public-key"}]}"#,
            r#"{"challenge": "AAAA", "allowCredentials": {"id": "AAAA"}}"#,
        ] {
            assert!(
                matches!(RequestBinary::of(&json(bad)), Err(TapFailed::Failed(_))),
                "{bad}"
            );
        }
    }

    /// The answer built by hand has `toJSON()`'s shape, which the server
    /// reads (`AssertionResponse`): binary members base64url, the optional
    /// ones only when present
    #[test]
    fn the_answer_is_built_in_to_jsons_shape() {
        let answer = Answer {
            id: "a2V5".into(),
            kind: "public-key".into(),
            raw_id: b"key".to_vec(),
            attachment: Some("platform".into()),
            extensions: serde_json::Map::new(),
            client_data_json: b"{}".to_vec(),
            authenticator_data: vec![0; 3],
            signature: vec![0x30, 0x45],
            user_handle: Some(b"user".to_vec()),
        };
        assert_eq!(
            answer.to_json().unwrap(),
            json(
                r#"{"id": "a2V5", "type": "public-key", "rawId": "a2V5",
                    "authenticatorAttachment": "platform",
                    "clientExtensionResults": {},
                    "response": {"clientDataJSON": "e30", "authenticatorData": "AAAA",
                                 "signature": "MEU", "userHandle": "dXNlcg"}}"#
            )
        );
        // No attachment and no user handle: left out
        let bare = Answer::default().to_json().unwrap();
        assert!(bare.get("authenticatorAttachment").is_none(), "{bare}");
        assert!(bare["response"].get("userHandle").is_none(), "{bare}");
        assert_eq!(bare["clientExtensionResults"], json("{}"));
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
