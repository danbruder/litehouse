use axum::{response::Html, routing::get, Router};

pub fn create_admin_router() -> Router {
    Router::new().route("/", get(|| async { Html("<h1>litehouse</h1><p>UI coming in v2.</p>") }))
}
