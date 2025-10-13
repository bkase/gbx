//! Static development server with COOP/COEP headers for SharedArrayBuffer support.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use axum::{
    http::{header, HeaderName, HeaderValue},
    routing::get_service,
    Router,
};
use clap::Parser;
use tokio::{net::TcpListener, signal};
use tower::ServiceBuilder;
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};
use tracing::{info, warn};
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Parser, Debug)]
#[command(author, version, about = "Static dev server with COOP/COEP headers")]
struct Args {
    /// Directory containing static assets (Trunk dist output)
    #[arg(long, default_value = "web/dist")]
    dist: PathBuf,

    /// Index file served as fallback for missing routes
    #[arg(long, default_value = "index.html")]
    index: PathBuf,

    /// Address to bind (ip or host)
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Port to bind
    #[arg(long, default_value_t = 8080)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let Args {
        dist,
        index,
        host,
        port,
    } = Args::parse();

    let dist_dir = canonicalize_or_create(&dist)
        .with_context(|| format!("failed to locate or create {dist:?}"))?;

    let index_path = resolve_index(&dist_dir, &index);

    let app = build_app(dist_dir.clone(), index_path.clone());

    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .context("failed to parse bind address")?;

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind listener on {addr}"))?;

    if let Some(index_path) = &index_path {
        info!(
            "serving {} with index {:?} on http://{}",
            dist_dir.display(),
            index_path,
            addr
        );
    } else {
        warn!(
            "serving {} without index fallback ({} missing) on http://{}",
            dist_dir.display(),
            index.display(),
            addr
        );
    }

    let server = axum::serve(listener, app.into_make_service());

    tokio::select! {
        result = server => result.context("server exited with error")?,
        _ = signal::ctrl_c() => {
            warn!("received Ctrl+C, shutting down");
        }
    }

    Ok(())
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info"));

    // Ignore error if already set (e.g., during tests).
    let _ = fmt().with_env_filter(env_filter).try_init();
}

fn canonicalize_or_create(path: &Path) -> Result<PathBuf> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    Ok(path.canonicalize()?)
}

fn resolve_index(dist_dir: &Path, index: &Path) -> Option<PathBuf> {
    let index_path = if index.is_absolute() {
        index.to_path_buf()
    } else {
        dist_dir.join(index)
    };

    if index_path.exists() {
        Some(index_path)
    } else {
        None
    }
}

fn build_app(dist_dir: PathBuf, index_path: Option<PathBuf>) -> Router {
    let assets = get_service(ServeDir::new(dist_dir).append_index_html_on_directories(true));

    let header_layer = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("cross-origin-opener-policy"),
            HeaderValue::from_static("same-origin"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("cross-origin-embedder-policy"),
            HeaderValue::from_static("require-corp"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("cross-origin-resource-policy"),
            HeaderValue::from_static("same-origin"),
        ))
        .layer(TraceLayer::new_for_http())
        .into_inner();

    let mut router = Router::new().nest_service("/", assets);

    if let Some(index) = index_path {
        let fallback = get_service(ServeFile::new(index));
        router = router.fallback_service(fallback);
    }

    router.layer(header_layer)
}
