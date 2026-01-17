use axum::{
    body::Body,
    http::{header, Request, Response, StatusCode},
    routing::get,
    Router,
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets/dist/"]
struct Assets;

/// Create the admin SPA router
pub fn create_admin_router() -> Router {
    Router::new().fallback(get(serve_spa))
}

/// Serve static files from embedded assets, falling back to index.html for SPA routing
async fn serve_spa(req: Request<Body>) -> Response<Body> {
    let path = req.uri().path().trim_start_matches('/');

    // Try to serve the exact file first
    if !path.is_empty() {
        if let Some(content) = Assets::get(path) {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(content.data.into_owned()))
                .unwrap();
        }
    }

    // Fall back to index.html for SPA routing
    match Assets::get("index.html") {
        Some(content) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html")
            .body(Body::from(content.data.into_owned()))
            .unwrap(),
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("Admin SPA not found. Run: cd assets && elm make src/Main.elm --output=dist/app.js && cp public/index.html dist/"))
            .unwrap(),
    }
}
