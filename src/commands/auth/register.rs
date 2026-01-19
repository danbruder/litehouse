use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::auth::jwt::{generate_access_token, generate_refresh_token, get_refresh_token_ttl_days};
use crate::db;
use crate::models::{
    AuthResponse, Organization, OrganizationMember, OrgRole, RefreshToken, TokenPair, User,
};

#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    #[error("User already exists with email: {0}")]
    UserAlreadyExists(String),
    #[error("Organization already exists with name: {0}")]
    OrganizationAlreadyExists(String),
    #[error("Failed to create user: {0}")]
    UserError(#[from] crate::models::UserError),
    #[error("Failed to create organization: {0}")]
    OrganizationError(#[from] crate::models::OrganizationError),
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
    #[error("JWT error: {0}")]
    JwtError(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, RegisterError>;

/// Register a new user with an optional organization
/// If org_name is provided, creates a new organization and makes the user an owner
/// If org_name is None, creates a default organization for the user
#[instrument(skip(pool, password, jwt_secret))]
pub async fn execute(
    pool: &Pool<Sqlite>,
    email: &str,
    password: &str,
    full_name: Option<String>,
    org_name: Option<String>,
    jwt_secret: &str,
) -> Result<AuthResponse> {
    // Check if user already exists
    if let Some(_) = db::user::get_by_email(pool, email).await? {
        return Err(RegisterError::UserAlreadyExists(email.to_string()));
    }

    // Determine organization name
    let organization_name = org_name.unwrap_or_else(|| format!("{}'s Organization", email));

    // Check if organization already exists
    if let Some(_) = db::organization::get_by_name(pool, &organization_name).await? {
        return Err(RegisterError::OrganizationAlreadyExists(
            organization_name,
        ));
    }

    // Create user
    let user = User::new(email, password, full_name)?;
    db::user::save(pool, &user).await?;

    info!("Created user '{}'", user.email);

    // Create organization
    let org = Organization::new(&organization_name)?;
    db::organization::save(pool, &org).await?;

    info!("Created organization '{}'", org.name);

    // Add user as organization owner
    let member = OrganizationMember::new(&org.id, &user.id, OrgRole::Owner);
    db::organization_member::save(pool, &member).await?;

    info!("Added user '{}' as owner of organization '{}'", user.email, org.name);

    // Generate tokens
    let access_token = generate_access_token(&user.id, &user.email, jwt_secret)?;
    let refresh_token_str = generate_refresh_token();
    let refresh_token = RefreshToken::new(&user.id, &refresh_token_str, get_refresh_token_ttl_days());
    db::refresh_token::save(pool, &refresh_token).await?;

    let tokens = TokenPair::new(access_token, refresh_token_str);

    // Get user organizations with roles
    let orgs = db::organization::get_user_organizations(pool, &user.id).await?;

    Ok(AuthResponse::new(user, tokens, orgs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;

    const TEST_JWT_SECRET: &str = "test-secret-for-registration";

    #[tokio::test]
    async fn test_register_happy_path() {
        let pool = get_test_pool().await;

        let response = execute(
            &pool,
            "test@example.com",
            "password123",
            Some("Test User".to_string()),
            Some("Test Org".to_string()),
            TEST_JWT_SECRET,
        )
        .await
        .unwrap();

        assert_eq!(response.user.email, "test@example.com");
        assert_eq!(response.user.full_name, Some("Test User".to_string()));
        assert!(!response.tokens.access_token.is_empty());
        assert!(!response.tokens.refresh_token.is_empty());
        assert_eq!(response.organizations.len(), 1);
        assert_eq!(response.organizations[0].organization.name, "Test Org");
        assert_eq!(response.organizations[0].role, OrgRole::Owner);
    }

    #[tokio::test]
    async fn test_register_default_org() {
        let pool = get_test_pool().await;

        let response = execute(
            &pool,
            "default@example.com",
            "password123",
            None,
            None,
            TEST_JWT_SECRET,
        )
        .await
        .unwrap();

        assert_eq!(response.organizations.len(), 1);
        assert!(response.organizations[0].organization.name.contains("default@example.com"));
        assert_eq!(response.organizations[0].role, OrgRole::Owner);
    }

    #[tokio::test]
    async fn test_register_user_already_exists() {
        let pool = get_test_pool().await;

        // Register once
        execute(
            &pool,
            "duplicate@example.com",
            "password123",
            None,
            Some("First Org".to_string()),
            TEST_JWT_SECRET,
        )
        .await
        .unwrap();

        // Try to register again
        let result = execute(
            &pool,
            "duplicate@example.com",
            "password123",
            None,
            Some("Second Org".to_string()),
            TEST_JWT_SECRET,
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            RegisterError::UserAlreadyExists(email) => {
                assert_eq!(email, "duplicate@example.com");
            }
            _ => panic!("Expected UserAlreadyExists error"),
        }
    }

    #[tokio::test]
    async fn test_register_weak_password() {
        let pool = get_test_pool().await;

        let result = execute(
            &pool,
            "weak@example.com",
            "short",
            None,
            None,
            TEST_JWT_SECRET,
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            RegisterError::UserError(_) => {}
            _ => panic!("Expected UserError for weak password"),
        }
    }
}
