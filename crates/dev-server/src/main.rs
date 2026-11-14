//! `scripts/dev.sh`'s server: the client's development loop.
//!
//! - Builds the dev bundle (`scripts/build.sh --dev`, unoptimized, no
//!   wasm-opt) and rebuilds it whenever the client's sources change, by
//!   polling their mtimes.
//! - Serves `/agora/app`, the page hosting the island, as the **upstream**
//!   serves it: its markup and its Content-Security-Policy header, verbatim,
//!   so the one copy of both stays agora-server's (`src/web/app.rs`,
//!   `templates/app.html`) and a CSP failure shows up here, not after a
//!   deploy. The only change is one dev script, loaded as a file (the CSP
//!   allows no inline script), that reloads the page after a rebuild.
//! - Serves `/agora/app/assets/*` from the local dev bundle.
//! - Proxies everything else (the API, the site's CSS and scripts, the
//!   plain pages the client links to) to the upstream, by default
//!   production.

use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    net::{Ipv4Addr, SocketAddr},
    path::{Component, Path as FsPath, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::{Path, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use futures_util::stream::{self, Stream};
use tokio::sync::watch;
use url::Url;

/// The page hosting the island, as agora-server routes it
const APP_PAGE: &str = "/agora/app";
/// What the client's mount element looks like in the page (the client's
/// `ROOT_ID`, agora-server's `templates/app.html`)
const MOUNT: &str = r#"id="agora-app""#;
/// The dev endpoints, under a prefix no agora route uses
const RELOAD_SCRIPT: &str = "/__agora-web-dev/reload.js";
const EVENTS: &str = "/__agora-web-dev/events";
/// The tag added to the page: a file, as the page's `script-src 'self'` needs
const RELOAD_TAG: &str = r#"<script type="module" src="/__agora-web-dev/reload.js"></script>"#;

const DEFAULT_PORT: u16 = 8088;
const DEFAULT_UPSTREAM: &str = "https://subliminal.technology";
/// How often the sources are checked for changes
const POLL: Duration = Duration::from_millis(400);
/// The largest request body passed on to the upstream
const MAX_BODY: usize = 16 * 1024 * 1024;

const USAGE: &str = "\
usage: scripts/dev.sh [--port PORT] [--upstream URL]

  --port PORT     where to listen, on 127.0.0.1 (default 8088,
                  or $AGORA_WEB_DEV_PORT)
  --upstream URL  the agora server whose page, CSS and API to use
                  (default https://subliminal.technology,
                  or $AGORA_WEB_DEV_UPSTREAM); for a local server,
                  http://127.0.0.1:8080
";

struct Config {
    port: u16,
    upstream: Url,
}

impl Config {
    fn from_args() -> Result<Self, String> {
        let mut port = match std::env::var("AGORA_WEB_DEV_PORT") {
            Ok(p) => p
                .parse()
                .map_err(|e| format!("AGORA_WEB_DEV_PORT={p}: {e}"))?,
            Err(_) => DEFAULT_PORT,
        };
        let mut upstream =
            std::env::var("AGORA_WEB_DEV_UPSTREAM").unwrap_or_else(|_| DEFAULT_UPSTREAM.into());
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--port" => {
                    let p = args.next().ok_or("--port needs a value")?;
                    port = p.parse().map_err(|e| format!("--port {p}: {e}"))?;
                }
                "--upstream" => upstream = args.next().ok_or("--upstream needs a value")?,
                "-h" | "--help" => {
                    print!("{USAGE}");
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
            }
        }
        let upstream = Url::parse(&upstream).map_err(|e| format!("upstream {upstream}: {e}"))?;
        if !matches!(upstream.scheme(), "http" | "https") || upstream.host_str().is_none() {
            return Err(format!("upstream {upstream}: not an http(s) origin"));
        }
        Ok(Self { port, upstream })
    }
}

/// The latest build, as the reload script sees it
#[derive(Clone, Debug, PartialEq)]
struct BuildStatus {
    /// Changes on every successful build and every server start, so a page
    /// reloads after either
    generation: String,
    /// Whether the latest build failed (the last good one is still served)
    failed: bool,
}

#[derive(Clone)]
struct AppState {
    upstream: Url,
    /// Whether cookies and `Authorization` cross the proxy: only to an
    /// upstream on this machine. Browsers don't separate cookies by port, so
    /// to a remote upstream (production) they would carry every local app's
    /// cookies out, and bring production's session cookie back into the
    /// jar every localhost port shares.
    credentials: bool,
    http: reqwest::Client,
    bundle: Arc<PathBuf>,
    status: watch::Receiver<BuildStatus>,
}

/// The workspace root: this crate is `crates/dev-server`
fn workspace_root() -> PathBuf {
    FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn main() {
    let config = match Config::from_args() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };
    let root = workspace_root();
    let bundle = root.join("target/dev-dist");

    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let (tx, rx) = watch::channel(BuildStatus {
        generation: format!("{started}-0"),
        failed: false,
    });
    // The first build before serving, so the first page load has a client
    let mut watcher = Watcher::new(root.clone(), bundle.clone());
    watcher.poll(&tx, started);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(POLL);
            watcher.poll(&tx, started);
        }
    });

    let state = AppState {
        credentials: is_loopback(&config.upstream),
        upstream: config.upstream.clone(),
        http: upstream_client(),
        bundle: Arc::new(bundle),
        status: rx,
    };
    let runtime = tokio::runtime::Runtime::new().expect("a tokio runtime");
    runtime.block_on(async move {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, config.port));
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => listener,
            Err(e) => {
                eprintln!("error: can't listen on {addr}: {e} (try --port)");
                std::process::exit(1);
            }
        };
        eprintln!(
            "agora-web dev: http://{addr}{APP_PAGE} (upstream {})",
            config.upstream
        );
        axum::serve(listener, app(state)).await.expect("serving");
    });
}

fn upstream_client() -> reqwest::Client {
    reqwest::Client::builder()
        // Redirects go back to the browser, which follows them through here
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(60))
        .user_agent("agora-web-dev")
        .build()
        .expect("an HTTP client")
}

fn app(state: AppState) -> Router {
    Router::new()
        .route(APP_PAGE, get(page))
        // agora-server serves the page at both
        .route("/agora/app/", get(page))
        .route("/agora/app/assets/{*path}", get(asset))
        .route(RELOAD_SCRIPT, get(reload_script))
        .route(EVENTS, get(events))
        .fallback(proxy)
        .layer(axum::middleware::from_fn(local_host_only))
        .with_state(state)
}

/// Whether `url`'s host is this machine
fn is_loopback(url: &Url) -> bool {
    url.host().is_some_and(|host| is_loopback_host(&host))
}

fn is_loopback_host<S: AsRef<str>>(host: &url::Host<S>) -> bool {
    match host {
        url::Host::Domain(domain) => domain.as_ref() == "localhost",
        url::Host::Ipv4(ip) => ip.is_loopback(),
        url::Host::Ipv6(ip) => ip.is_loopback(),
    }
}

/// Whether a `Host` header (`name[:port]`, `[v6]:port`) names this machine
fn is_loopback_host_header(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(v6) => v6.split_once(']').map(|(ip, _)| ip),
        None => Some(host.rsplit_once(':').map_or(host, |(name, _)| name)),
    };
    name.and_then(|name| match name.parse::<std::net::Ipv6Addr>() {
        Ok(ip) => Some(url::Host::Ipv6(ip)),
        Err(_) => url::Host::parse(name).ok(),
    })
    .is_some_and(|host| is_loopback_host(&host))
}

/// Refuses a request whose `Host` isn't this machine: a page elsewhere
/// that rebinds its own name to 127.0.0.1 (DNS rebinding) can't use the
/// proxy to reach the upstream from here
async fn local_host_only(request: Request, next: axum::middleware::Next) -> Response {
    let local = request
        .headers()
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
        .is_some_and(is_loopback_host_header);
    if local {
        next.run(request).await
    } else {
        (
            StatusCode::FORBIDDEN,
            "agora-web dev server: local hosts only\n",
        )
            .into_response()
    }
}

// -- Watching and building --

/// Rebuilds the bundle when the client's sources change
struct Watcher {
    root: PathBuf,
    bundle: PathBuf,
    last: Option<u64>,
    builds: u64,
}

impl Watcher {
    fn new(root: PathBuf, bundle: PathBuf) -> Self {
        Self {
            root,
            bundle,
            last: None,
            builds: 0,
        }
    }

    /// What a change to the client means: its crate (sources, assets,
    /// manifest, build script) and the workspace's manifest and lockfile
    fn watched(&self) -> [PathBuf; 3] {
        [
            self.root.join("crates/agora-web"),
            self.root.join("Cargo.toml"),
            self.root.join("Cargo.lock"),
        ]
    }

    fn poll(&mut self, tx: &watch::Sender<BuildStatus>, started: u128) {
        let mut stamp = fingerprint(&self.watched());
        if self.last == Some(stamp) {
            return;
        }
        // Let an editor finish saving (several files, or write-and-rename)
        loop {
            std::thread::sleep(Duration::from_millis(150));
            let again = fingerprint(&self.watched());
            if again == stamp {
                break;
            }
            stamp = again;
        }
        self.last = Some(stamp);
        let ok = self.build();
        if ok {
            self.builds += 1;
        }
        tx.send_modify(|status| {
            if ok {
                status.generation = format!("{started}-{}", self.builds);
            }
            status.failed = !ok;
        });
    }

    /// `scripts/build.sh --dev` into a fresh directory, then swapped in, so
    /// a failed build leaves the last good bundle in place
    fn build(&self) -> bool {
        let next = self.bundle.with_extension("next");
        eprintln!("agora-web dev: building...");
        let status = Command::new(self.root.join("scripts/build.sh"))
            .arg("--dev")
            .arg(&next)
            .status();
        match status {
            Ok(s) if s.success() => {
                let _ = fs::remove_dir_all(&self.bundle);
                match fs::rename(&next, &self.bundle) {
                    Ok(()) => {
                        eprintln!("agora-web dev: built; reloading open pages");
                        true
                    }
                    Err(e) => {
                        eprintln!("agora-web dev: can't move the bundle into place: {e}");
                        false
                    }
                }
            }
            Ok(s) => {
                eprintln!("agora-web dev: the build failed ({s}); still serving the last good one");
                false
            }
            Err(e) => {
                eprintln!("agora-web dev: can't run scripts/build.sh: {e}");
                false
            }
        }
    }
}

/// A hash of every file's path, size and mtime under `paths`
fn fingerprint(paths: &[PathBuf]) -> u64 {
    fn walk(path: &FsPath, hasher: &mut DefaultHasher) {
        // The link itself, never its target: no symlink loops
        let Ok(meta) = fs::symlink_metadata(path) else {
            return;
        };
        if meta.is_dir() {
            let Ok(entries) = fs::read_dir(path) else {
                return;
            };
            let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            entries.sort();
            for entry in entries {
                // Editors' swap and backup files aren't sources
                let name = entry.file_name().unwrap_or_default().to_string_lossy();
                if name.starts_with('.') || name.ends_with('~') {
                    continue;
                }
                walk(&entry, hasher);
            }
        } else {
            path.hash(hasher);
            meta.len().hash(hasher);
            meta.modified().ok().hash(hasher);
        }
    }
    let mut hasher = DefaultHasher::new();
    for path in paths {
        walk(path, &mut hasher);
    }
    hasher.finish()
}

// -- Routes --

/// `GET /agora/app`: the upstream's page and CSP, plus the reload script
async fn page(State(state): State<AppState>, mut headers: HeaderMap) -> Response {
    // The page is rewritten here, so always a whole one: never a 304
    headers.remove(header::IF_NONE_MATCH);
    headers.remove(header::IF_MODIFIED_SINCE);
    let url = match upstream_url(&state.upstream, APP_PAGE) {
        Ok(url) => url,
        Err(response) => return response,
    };
    let upstream = match send(&state, reqwest::Method::GET, url, &headers, Bytes::new()).await {
        Ok(upstream) => upstream,
        Err(response) => return response,
    };
    let status = upstream.status();
    let Some(csp) = upstream
        .headers()
        .get(header::CONTENT_SECURITY_POLICY)
        .cloned()
    else {
        return bad_gateway(
            "the upstream's /agora/app has no Content-Security-Policy header; \
             the dev server serves the page only under the upstream's own CSP",
        );
    };
    let html = match upstream.text().await {
        Ok(html) => html,
        Err(e) => return bad_gateway(&format!("reading the upstream's page: {e}")),
    };
    if !status.is_success() {
        return bad_gateway(&format!("the upstream's /agora/app answered {status}"));
    }
    if !html.contains(MOUNT) {
        return bad_gateway(&format!(
            "the upstream's /agora/app has no <div {MOUNT}> to mount \
             into. A local agora-server built without the client serves a \
             placeholder there: run `just web-build` in agora and rebuild the \
             server (any build will do; the assets come from here)."
        ));
    }
    (
        [
            (header::CONTENT_SECURITY_POLICY, csp),
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        with_reload_script(&html),
    )
        .into_response()
}

/// `html` with the reload script's tag before `</body>` (or at the end)
fn with_reload_script(html: &str) -> String {
    let at = html.rfind("</body>").unwrap_or(html.len());
    let mut out = String::with_capacity(html.len() + RELOAD_TAG.len() + 1);
    out.push_str(&html[..at]);
    out.push_str(RELOAD_TAG);
    out.push('\n');
    out.push_str(&html[at..]);
    out
}

/// `GET /agora/app/assets/{*path}`: a file of the local dev bundle
async fn asset(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    let relative = FsPath::new(&path);
    if !relative
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(state.bundle.join(relative)).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, mime_type(&path)),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn mime_type(path: &str) -> HeaderValue {
    let ext = path
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or_default();
    HeaderValue::from_static(match ext {
        "js" | "mjs" => "text/javascript",
        "wasm" => "application/wasm",
        "css" => "text/css",
        "html" => "text/html; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        _ => "application/octet-stream",
    })
}

async fn reload_script() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("reload.js"),
    )
        .into_response()
}

/// `GET /__agora-web-dev/events`: the build status now, then on each change
async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let mut rx = state.status.clone();
    rx.mark_changed();
    let stream = stream::unfold(rx, |mut rx| async move {
        rx.changed().await.ok()?;
        let status = rx.borrow_and_update().clone();
        let event = Event::default()
            .event(if status.failed { "failed" } else { "built" })
            .data(status.generation);
        Some((Ok(event), rx))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Everything else: passed to the upstream as it came, and its answer back
async fn proxy(State(state): State<AppState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let path = parts
        .uri
        .path_and_query()
        .map_or("/", |path_and_query| path_and_query.as_str());
    let url = match upstream_url(&state.upstream, path) {
        Ok(url) => url,
        Err(response) => return response,
    };
    let body = match to_bytes(body, MAX_BODY).await {
        Ok(body) => body,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let upstream = match send(&state, parts.method, url, &parts.headers, body).await {
        Ok(upstream) => upstream,
        Err(response) => return response,
    };
    let status = upstream.status();
    let headers = response_headers(upstream.headers(), &state.upstream, state.credentials);
    match upstream.bytes().await {
        Ok(bytes) => (status, headers, bytes).into_response(),
        Err(e) => bad_gateway(&format!("reading the upstream's answer: {e}")),
    }
}

/// `path_and_query` on the upstream; refused if it would leave the
/// upstream's origin (a `//host/...` path). The error is the response to
/// send, as handlers return it; its size doesn't matter here.
#[allow(clippy::result_large_err)]
fn upstream_url(upstream: &Url, path_and_query: &str) -> Result<Url, Response> {
    match upstream.join(path_and_query) {
        Ok(url) if url.origin() == upstream.origin() => Ok(url),
        _ => Err(StatusCode::BAD_REQUEST.into_response()),
    }
}

/// Sends a request to the upstream; the error is a 502 to send back
#[allow(clippy::result_large_err)]
async fn send(
    state: &AppState,
    method: reqwest::Method,
    url: Url,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<reqwest::Response, Response> {
    let mut forwarded = HeaderMap::new();
    let connection = connection_headers(headers);
    for (name, value) in headers {
        if !is_hop_by_hop(name)
            && !connection.contains(name)
            && (state.credentials
                || (*name != header::COOKIE && *name != header::AUTHORIZATION))
            && *name != header::HOST
            && *name != header::CONTENT_LENGTH
            // The body comes back as sent; this server doesn't decode it
            && *name != header::ACCEPT_ENCODING
        {
            forwarded.append(name.clone(), value.clone());
        }
    }
    state
        .http
        .request(method, url.clone())
        .headers(forwarded)
        .body(body)
        .send()
        .await
        .map_err(|e| bad_gateway(&format!("{url}: {e}")))
}

/// The upstream's response headers, less those about its own connection,
/// with its absolute redirects made relative so the browser stays here
/// (and `Set-Cookie` only when `credentials`)
fn response_headers(headers: &HeaderMap, upstream: &Url, credentials: bool) -> HeaderMap {
    let mut out = HeaderMap::new();
    let connection = connection_headers(headers);
    for (name, value) in headers {
        if is_hop_by_hop(name)
            || connection.contains(name)
            || (!credentials && *name == header::SET_COOKIE)
            || *name == header::CONTENT_LENGTH
            // Over plain http on 127.0.0.1 it means nothing, and is unwanted
            || *name == header::STRICT_TRANSPORT_SECURITY
        {
            continue;
        }
        let mut value = value.clone();
        if *name == header::LOCATION
            && let Some(local) = value
                .to_str()
                .ok()
                .and_then(|location| Url::parse(location).ok())
                .filter(|location| location.origin() == upstream.origin())
                .and_then(|location| {
                    HeaderValue::from_str(&location[url::Position::BeforePath..]).ok()
                })
        {
            value = local;
        }
        out.append(name.clone(), value);
    }
    out
}

/// The headers a `Connection` header names, which are hop-by-hop too
fn connection_headers(headers: &HeaderMap) -> Vec<HeaderName> {
    headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| HeaderName::try_from(name.trim()).ok())
        .collect()
}

fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn bad_gateway(why: &str) -> Response {
    eprintln!("agora-web dev: {why}");
    (
        StatusCode::BAD_GATEWAY,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        Body::from(format!("agora-web dev server: {why}\n")),
    )
        .into_response()
}

#[cfg(test)]
mod tests;
