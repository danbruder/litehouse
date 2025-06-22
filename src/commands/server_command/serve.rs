use anyhow::{Context, Result};
use hyper::body::to_bytes;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::util::ServiceExt;
use tracing::{error, info, instrument};

use crate::api;
use crate::config::ServerConfig;
use crate::db;
use crate::models::AppState;

// Message types for communication between servers
#[derive(Debug, Clone, Serialize, Deserialize)]
enum ApiRequest {
    ListApps,
    GetApp { name: String },
    CreateApp { name: String },
    DeleteApp { id: String },
    StartApp { id: String },
    StopApp { id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum ApiResponse {
    Apps(Vec<AppInfo>),
    App(Option<AppInfo>),
    Success(String),
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppInfo {
    id: String,
    name: String,
    state: String,
}

/// Shared state for the proxy server
pub struct ProxyState {
    pub db_pool: sqlx::Pool<sqlx::Sqlite>,
}

/// Start the BinaryDrop server
#[instrument]
pub async fn execute(config: ServerConfig) -> Result<()> {
    // Connect to database
    let pool = db::init_pool().await?;
    // TODO: Initialize supervisor when available
    info!("Starting server without supervisor (not yet implemented)");

    // Create shared state
    let proxy_state = Arc::new(RwLock::new(ProxyState {
        db_pool: pool.clone(),
    }));

    // Parse host and port for proxy server
    let addr: SocketAddr = format!("{}:{}", config.host, config.proxy_port)
        .parse()
        .context(format!(
            "Invalid host or port: {}:{}",
            config.host, config.proxy_port
        ))?;

    // Create service for proxy server
    let make_svc = make_service_fn(move |_conn| {
        let state = Arc::clone(&proxy_state);
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                let state = Arc::clone(&state);
                async move { handle_request(req, state).await }
            }))
        }
    });

    // Create proxy server
    let server = Server::bind(&addr).serve(make_svc);

    info!(
        "Starting BinaryDrop proxy server on http://{}:{}",
        config.host, config.proxy_port
    );
    println!(
        "BinaryDrop proxy server running at http://{}:{}",
        config.host, config.proxy_port
    );
    println!("Press Ctrl+C to stop");

    // Run proxy server
    tokio::select! {
        result = server => {
            result.context("Proxy server error")?;
        }
        _ = tokio::signal::ctrl_c() => {
            println!("Shutting down...");
        }
    }

    println!("API server stopped");

    Ok(())
}

/// Handle incoming requests to the proxy server
async fn handle_request(
    req: Request<Body>,
    state: Arc<RwLock<ProxyState>>,
) -> Result<Response<Body>, Infallible> {
    let headers = req.headers().clone();
    let host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    if host.starts_with("admin-api.") {
        let api_router = api::create_api_router(Arc::clone(&state));
        let response = api_router.oneshot(req).await.unwrap();
        let (parts, body) = response.into_parts();
        let body_bytes = to_bytes(body).await.unwrap_or_default();
        Ok(Response::from_parts(parts, Body::from(body_bytes)))
    } else if host.starts_with("admin.") {
        // Regular admin interface
        Ok(admin_interface(state).await)
    } else {
        // Extract app name from host
        let app_name = host.split('.').next().unwrap_or("");
        if app_name.is_empty() {
            return Ok(Response::builder()
                .status(404)
                .body(Body::from("No app specified in host header"))
                .unwrap());
        }

        // Proxy to app
        match proxy_to_app(state, app_name, req).await {
            Ok(response) => Ok(response),
            Err(e) => {
                error!("Proxy error: {}", e);
                Ok(Response::builder()
                    .status(500)
                    .body(Body::from(format!("Proxy error: {}", e)))
                    .unwrap())
            }
        }
    }
}

/// Admin interface handler
async fn admin_interface(state: Arc<RwLock<ProxyState>>) -> Response<Body> {
    let pool = state.read().await.db_pool.clone();
    let apps = db::apps::get_all(&pool).await.unwrap_or_else(|_| vec![]);

    // Build HTML response
    let mut html = String::from(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>BinaryDrop Admin</title>
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; margin: 0; padding: 20px; }
        h1 { color: #333; }
        table { border-collapse: collapse; width: 100%; }
        th, td { text-align: left; padding: 8px; }
        tr:nth-child(even) { background-color: #f2f2f2; }
        th { background-color: #4CAF50; color: white; }
        .running { color: green; }
        .stopped { color: red; }
        .created { color: blue; }
    </style>
</head>
<body>
    <h1>BinaryDrop Admin</h1>
    <h2>Apps</h2>
    <table>
        <tr>
            <th>Name</th>
            <th>Status</th>
            <th>Port</th>
            <th>PID</th>
            <th>URL</th>
        </tr>
"#,
    );

    // Add rows for each app
    for app in apps {
        let status_class = match app.state {
            AppState::Running => "running",
            AppState::Stopped => "stopped",
            _ => "created",
        };

        html.push_str(&format!(
            r#"<tr>
            <td>{}</td>
            <td class="{}">{}</td>
        </tr>"#,
            app.name, status_class, app.state,
        ));
    }

    // Add API information
    html.push_str(
        r#"
    </table>
</body>
</html>
"#,
    );

    Response::builder()
        .header("Content-Type", "text/html")
        .body(Body::from(html))
        .unwrap()
}

/// Proxy request to app
async fn proxy_to_app(
    _state: Arc<RwLock<ProxyState>>,
    _app_name: &str,
    _req: Request<Body>,
) -> anyhow::Result<Response<Body>> {
    // let pool = state.read().await.db_pool.clone();
    // let app = db::apps::get_by_name(&pool, app_name)
    //     .await?
    //     .ok_or_else(|| anyhow::anyhow!("App '{}' not found", app_name))?;

    // // Check if app is running
    // if app.state != AppState::Running {
    //     return Ok(Response::builder()
    //         .status(503)
    //         .body(Body::from(format!("App '{}' is not running", app_name)))
    //         .unwrap());
    // }

    // // Create URI for proxying
    // let path_and_query = req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("");
    // let uri = format!(
    //     "http://{}:{}{}",
    //     app.host,
    //     app.port.map_or("".to_string(), |p| p.to_string()),
    //     path_and_query
    // );

    // // Create new request
    // let (parts, body) = req.into_parts();
    // let mut new_req = Request::builder().method(parts.method).uri(uri);

    // // Copy headers
    // for (name, value) in parts.headers.iter() {
    //     if name != "host" {
    //         new_req = new_req.header(name, value);
    //     }
    // }

    // // Add custom headers
    // if let Some(host) = parts.headers.get("host") {
    //     new_req = new_req.header("X-Forwarded-Host", host);
    // }
    // new_req = new_req.header("X-Forwarded-Proto", "http");

    // // Build request
    // let new_req = new_req.body(body).context("Failed to build request")?;

    // // Send request
    // let client = Client::new();
    // let resp = client
    //     .request(new_req)
    //     .await
    //     .context("Proxy request failed")?;

    // Ok(resp)
    anyhow::bail!("Proxying to app is not implemented yet");
}
