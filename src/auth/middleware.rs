use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::instrument;

use crate::commands::server::AppState;

/// Extension type to store authenticated user information in request
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: String,
    pub email: String,
}

/// Middleware to authenticate requests using JWT
#[instrument(skip(state, request, next))]
pub async fn auth_middleware(
    State(state): State<Arc<RwLock<AppState>>>,
    mut request: Request<Body>,
    next: Next<Body>,
) -> Result<Response, AuthError> {
    // Extract the Authorization header
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .ok_or(AuthError::MissingToken)?;

    // Extract the token from "Bearer <token>"
    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(AuthError::InvalidToken)?;

    // Get JWT secret from state
    let jwt_secret = state.read().await.jwt_secret.clone();

    // Verify the token
    let claims = super::jwt::verify_access_token(token, &jwt_secret)
        .map_err(|_| AuthError::InvalidToken)?;

    // Check if token is expired
    if claims.is_expired() {
        return Err(AuthError::ExpiredToken);
    }

    // Insert authenticated user into request extensions
    let auth_user = AuthUser {
        user_id: claims.sub,
        email: claims.email,
    };

    request.extensions_mut().insert(auth_user);

    // Continue to the next middleware/handler
    Ok(next.run(request).await)
}

/// Errors that can occur during authentication
#[derive(Debug)]
pub enum AuthError {
    MissingToken,
    InvalidToken,
    ExpiredToken,
    Unauthorized,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AuthError::MissingToken => (StatusCode::UNAUTHORIZED, "Missing authorization token"),
            AuthError::InvalidToken => (StatusCode::UNAUTHORIZED, "Invalid authorization token"),
            AuthError::ExpiredToken => (StatusCode::UNAUTHORIZED, "Token has expired"),
            AuthError::Unauthorized => (StatusCode::FORBIDDEN, "Insufficient permissions"),
        };

        (status, message).into_response()
    }
}

/// Helper to extract authenticated user from request extensions
/// This is used in handlers that require authentication
pub fn get_auth_user(
    extensions: &axum::http::Extensions,
) -> Result<AuthUser, AuthError> {
    extensions
        .get::<AuthUser>()
        .cloned()
        .ok_or(AuthError::Unauthorized)
}
