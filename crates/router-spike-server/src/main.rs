//! SPIKE: a server for the router spike, without a database.
//!
//!     router-spike-server <port> <base> <dist dir> <site.css>
//!
//! `<base>` is `/agora` (today's mount) or `` (the root). Any GET below the
//! base that `Route` reads is pre-rendered through the router with the
//! fixture data it names (200); anything else below the base is a 404 page.
//! `/agora/api/social/feed` and `/agora/api/content/<id>` answer from the
//! same fixtures, and every API read is logged and counted
//! (`/__spike/api-reads`).

use std::{
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use agora_agentkit::{
    fixtures,
    ids::PostId,
    links::AgoraLink,
    names::CommunityName,
    requests::GetFeedInput,
    responses::{ContentResponse, PostResponse, PostWithCommentsResponse},
    ui::{
        feed::FeedPage,
        page::Page,
        route::{Route, ssr},
        thread::ThreadPage,
    },
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};

const CSP: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; \
     style-src 'self' 'unsafe-inline'; img-src 'self'; connect-src 'self'; base-uri 'self'";

#[derive(Clone)]
struct App {
    base: String,
    dist: PathBuf,
    css: PathBuf,
    api_reads: Arc<AtomicUsize>,
}

/// Six posts: three in `meta-governance`, three in `science`
fn posts() -> Vec<PostResponse> {
    (1..=6u128)
        .map(|n| {
            let mut post = fixtures::post();
            post.id = format!("5b1ce000-0000-4000-8000-{n:012x}").parse().unwrap();
            post.title = format!("Fixture post {n}");
            post.body = format!("The body of fixture post {n}.");
            if n > 3 {
                post.community_name = "science".parse::<CommunityName>().unwrap();
            }
            post
        })
        .collect()
}

fn feed(input: &GetFeedInput) -> Vec<PostResponse> {
    let mut posts: Vec<_> = posts()
        .into_iter()
        .filter(|p| {
            input
                .community
                .as_ref()
                .is_none_or(|c| *c == p.community_name)
        })
        .collect();
    if input.sort.is_some() {
        posts.reverse();
    }
    posts
}

fn thread(id: PostId) -> Option<PostWithCommentsResponse> {
    let post = posts().into_iter().find(|p| p.id == id)?;
    let mut read = fixtures::post_with_comments();
    read.post = post;
    Some(read)
}

/// The page the server reads for a route (the data block's)
fn read(route: &Route) -> Option<Page> {
    match &route.0 {
        AgoraLink::Feed(input) => {
            Some(FeedPage::new(input.clone(), feed(input).into_iter().collect()).into())
        }
        AgoraLink::Content { .. } => {
            let s = route.to_string();
            let id: PostId = s.strip_prefix("/post/")?.parse().ok()?;
            Some(ThreadPage::loaded(thread(id)?).into())
        }
        _ => None,
    }
}

fn document(app: &App, title: &str, body: &str) -> String {
    let base = &app.base;
    format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8"><meta name="agora-base" content="{base}">
<title>{title}</title><link rel="stylesheet" href="/__spike/site.css"></head>
<body><nav><a href="{base}/feed">Feed</a> · <a href="{base}/about">About (a server page)</a></nav><main>{body}</main><div style="height:3000px"></div><p id="far">far below</p><div style="height:3000px"></div>
<script type="module" src="/__spike/assets/loader.js"></script></body></html>"#
    )
}

async fn page(State(app): State<App>, uri: Uri) -> Response {
    let full = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let below = if app.base.is_empty() {
        full.to_string()
    } else {
        match full.strip_prefix(&app.base) {
            Some(rest) => rest.to_string(),
            None => return (StatusCode::NOT_FOUND, "not under the base").into_response(),
        }
    };
    let below = if below.is_empty() { "/".into() } else { below };
    // The 404 is decided here, before anything is drawn
    let route = match ssr::route(&below) {
        Ok(route) => route,
        Err(e) => {
            println!("GET {full} -> 404 ({e})");
            let body = document(&app, "Not found", "<h1>Not found</h1><p>No page here.</p>");
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_SECURITY_POLICY, CSP)],
                Html(body),
            )
                .into_response();
        }
    };
    let first = read(&route);
    match ssr::render(&app.base, &below, first) {
        Ok(r) => {
            println!("GET {full} -> 200 ({route})");
            let body = format!("{}<div id=\"agora-app\">{}</div>", r.script, r.html);
            (
                [(header::CONTENT_SECURITY_POLICY, CSP)],
                Html(document(&app, &route.to_string(), &body)),
            )
                .into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn api_feed(
    State(app): State<App>,
    Query(input): Query<GetFeedInput>,
) -> Json<Vec<PostResponse>> {
    let n = app.api_reads.fetch_add(1, Ordering::SeqCst) + 1;
    println!("API #{n}: feed {input:?}");
    Json(feed(&input))
}

async fn api_content(State(app): State<App>, Path(id): Path<String>) -> Response {
    let n = app.api_reads.fetch_add(1, Ordering::SeqCst) + 1;
    println!("API #{n}: content {id}");
    match id.parse::<PostId>().ok().and_then(thread) {
        Some(read) => Json(ContentResponse::Post(read)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn api_reads(State(app): State<App>) -> String {
    app.api_reads.load(Ordering::SeqCst).to_string()
}

async fn asset(State(app): State<App>, Path(name): Path<String>) -> Response {
    if name.contains("..") || name.starts_with('/') {
        return StatusCode::NOT_FOUND.into_response();
    }
    let kind = match name.rsplit('.').next() {
        Some("js") => "application/javascript",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    };
    match tokio::fs::read(app.dist.join(&name)).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, kind)], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn css(State(app): State<App>) -> Response {
    match tokio::fs::read(&app.css).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "text/css")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let port: u16 = args[1].parse().unwrap();
    let app = App {
        base: args[2].trim_end_matches('/').to_string(),
        dist: args[3].clone().into(),
        css: args[4].clone().into(),
        api_reads: Arc::default(),
    };
    // The API and the spike's own files first; the router's pages are the
    // fallback, so at the root they can't swallow `/agora/api`
    let router = Router::new()
        .route("/agora/api/social/feed", get(api_feed))
        .route("/agora/api/content/{id}", get(api_content))
        .route(
            "/agora/api/portal/session",
            get(|| async { StatusCode::UNAUTHORIZED }),
        )
        .route("/__spike/api-reads", get(api_reads))
        .route("/__spike/site.css", get(css))
        .route("/__spike/assets/{*name}", get(asset))
        .fallback(get(page))
        .with_state(app.clone());
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    println!("serving base {:?} on {addr}", app.base);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, router).await.unwrap();
}
