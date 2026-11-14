use super::*;

/// The CSP the fake upstream sends: production's, as agora-server's
/// `APP_CSP` reads (src/web/app.rs). The dev server must pass whatever the
/// upstream sends through unchanged; this copy is only test data.
const CSP: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; \
     style-src 'self' 'unsafe-inline'; img-src 'self'; connect-src 'self'; \
     frame-ancestors 'self'; base-uri 'self'; form-action 'self'";

const PAGE: &str = "<html><body><div id=\"agora-app\"></div>\
     <script type=\"module\" src=\"/agora/app/assets/loader.js\"></script></body></html>";

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Serve `router` on a free port of 127.0.0.1; its origin
async fn serve(router: Router) -> Url {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Url::parse(&format!("http://{addr}")).unwrap()
}

/// An upstream standing in for agora-server: the app page (or, with
/// `built: false`, the placeholder without a mount), an API route, and a
/// redirect to its own origin
async fn fake_upstream(built: bool) -> Url {
    let page = if built {
        PAGE
    } else {
        "<html><body>The web client isn't built</body></html>"
    };
    let router = Router::new()
        .route(
            APP_PAGE,
            get(move || async move { ([(header::CONTENT_SECURITY_POLICY, CSP)], page) }),
        )
        .route(
            "/agora/api/social/feed",
            get(|request: Request| async move {
                let query = request.uri().query().unwrap_or_default().to_owned();
                (
                    [
                        (header::CONTENT_TYPE, "application/json"),
                        (header::STRICT_TRANSPORT_SECURITY, "max-age=1"),
                    ],
                    format!("{{\"query\":\"{query}\"}}"),
                )
            }),
        )
        .route(
            "/agora/old",
            get(|State(origin): State<Url>| async move {
                (
                    StatusCode::FOUND,
                    [(
                        header::LOCATION,
                        origin.join("/agora/new?x=1").unwrap().to_string(),
                    )],
                )
            }),
        );
    // The redirect needs its own origin, known once bound
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let origin = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let router = router.with_state(origin.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    origin
}

/// A dev server in front of `upstream`, serving a bundle of one file
async fn dev_server(upstream: Url, name: &str) -> Url {
    let bundle =
        std::env::temp_dir().join(format!("agora-web-dev-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&bundle);
    fs::create_dir_all(bundle.join("snippets")).unwrap();
    fs::write(bundle.join("loader.js"), "// loader").unwrap();
    fs::write(bundle.join("snippets/x.wasm"), b"\0asm").unwrap();
    let (tx, rx) = watch::channel(BuildStatus {
        generation: "g-1".into(),
        failed: false,
    });
    // Keep the sender alive for the test's length
    std::mem::forget(tx);
    serve(app(AppState {
        upstream,
        http: upstream_client(),
        bundle: Arc::new(bundle),
        status: rx,
    }))
    .await
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

#[test]
fn the_page_is_the_upstreams_under_its_csp_with_the_reload_script() {
    runtime().block_on(async {
        let dev = dev_server(fake_upstream(true).await, "page").await;
        let response = client()
            .get(dev.join(APP_PAGE).unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()[header::CONTENT_SECURITY_POLICY], CSP);
        let html = response.text().await.unwrap();
        assert!(html.contains(MOUNT), "{html}");
        assert!(html.contains("/agora/app/assets/loader.js"), "{html}");
        assert!(html.contains(&format!("{RELOAD_TAG}\n</body>")), "{html}");
        let slash = client()
            .get(dev.join("/agora/app/").unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(slash.headers()[header::CONTENT_SECURITY_POLICY], CSP);
        assert!(slash.text().await.unwrap().contains(RELOAD_TAG));
    });
}

#[test]
fn a_placeholder_page_is_refused_not_served() {
    runtime().block_on(async {
        let dev = dev_server(fake_upstream(false).await, "placeholder").await;
        let response = client()
            .get(dev.join(APP_PAGE).unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 502);
        assert!(response.text().await.unwrap().contains("just web-build"));
    });
}

#[test]
fn assets_come_from_the_local_bundle() {
    runtime().block_on(async {
        let dev = dev_server(fake_upstream(true).await, "assets").await;
        let http = client();
        let loader = http
            .get(dev.join("/agora/app/assets/loader.js").unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(loader.status(), 200);
        assert_eq!(loader.headers()[header::CONTENT_TYPE], "text/javascript");
        assert_eq!(loader.text().await.unwrap(), "// loader");
        let wasm = http
            .get(dev.join("/agora/app/assets/snippets/x.wasm").unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(wasm.headers()[header::CONTENT_TYPE], "application/wasm");
        for missing in [
            "/agora/app/assets/nope.js",
            "/agora/app/assets/%2E%2E/%2E%2E/Cargo.toml",
            "/agora/app/assets/..%2F..%2FCargo.toml",
        ] {
            let response = http.get(dev.join(missing).unwrap()).send().await.unwrap();
            assert_eq!(response.status(), 404, "{missing}");
        }
        let script = http
            .get(dev.join(RELOAD_SCRIPT).unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(script.headers()[header::CONTENT_TYPE], "text/javascript");
        assert!(script.text().await.unwrap().contains(EVENTS));
    });
}

#[test]
fn the_rest_is_proxied_with_redirects_kept_here() {
    runtime().block_on(async {
        let dev = dev_server(fake_upstream(true).await, "proxy").await;
        let http = client();
        let feed = http
            .get(dev.join("/agora/api/social/feed?limit=3").unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(feed.status(), 200);
        assert_eq!(feed.headers()[header::CONTENT_TYPE], "application/json");
        assert!(
            feed.headers()
                .get(header::STRICT_TRANSPORT_SECURITY)
                .is_none()
        );
        assert_eq!(feed.text().await.unwrap(), r#"{"query":"limit=3"}"#);

        let old = http
            .get(dev.join("/agora/old").unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(old.status(), 302);
        assert_eq!(old.headers()[header::LOCATION], "/agora/new?x=1");
    });
}

#[test]
fn a_path_cannot_leave_the_upstream() {
    let upstream = Url::parse("https://subliminal.technology").unwrap();
    assert!(upstream_url(&upstream, "//example.com/x").is_err());
    assert_eq!(
        upstream_url(&upstream, "/agora/api/social/feed?limit=1")
            .unwrap()
            .as_str(),
        "https://subliminal.technology/agora/api/social/feed?limit=1"
    );
}

#[test]
fn the_reload_tag_goes_before_the_last_body_close() {
    assert_eq!(
        with_reload_script("<body>a</body></html>"),
        format!("<body>a{RELOAD_TAG}\n</body></html>")
    );
    assert_eq!(with_reload_script("x"), format!("x{RELOAD_TAG}\n"));
}

#[test]
fn the_fingerprint_sees_a_change() {
    let dir = std::env::temp_dir().join(format!("agora-web-dev-fp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/main.rs"), "a").unwrap();
    let paths = [dir.clone()];
    let before = fingerprint(&paths);
    assert_eq!(before, fingerprint(&paths));
    fs::write(dir.join("src/main.rs"), "ab").unwrap();
    assert_ne!(before, fingerprint(&paths));
    let _ = fs::remove_dir_all(&dir);
}
