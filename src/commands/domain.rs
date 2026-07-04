use anyhow::Result;
use bollard::Docker;
use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::caddy;
use crate::db;
use crate::models::is_valid_domain;

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error(
        "Invalid domain: {0}. Domains must be lowercase, contain a '.', and have no scheme, path, or spaces."
    )]
    InvalidDomain(String),
    #[error("Domain not found on app '{0}': {1}")]
    DomainNotFound(String, String),
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
}

type Result_<T> = Result<T, DomainError>;

/// Add a custom top-level domain to `app_name`'s route, alongside its
/// derived `{name}.{server_domain}` host, and resync Caddy.
#[instrument(skip(pool, docker))]
pub async fn add(pool: &Pool<Sqlite>, docker: &Docker, app_name: &str, domain: &str) -> Result_<()> {
    let domain = domain.trim();
    if !is_valid_domain(domain) {
        return Err(DomainError::InvalidDomain(domain.to_string()));
    }

    let mut app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| DomainError::AppNotFound(app_name.to_string()))?;

    let mut domains = app.custom_domains_list();
    if !domains.iter().any(|d| d == domain) {
        domains.push(domain.to_string());
    }
    app.custom_domains = Some(serde_json::to_string(&domains).expect("Vec<String> always serializes"));

    db::app::save(pool, &app).await?;
    info!("Added custom domain '{}' to app '{}'", domain, app_name);

    if let Err(e) = caddy::sync_configuration(docker, pool).await {
        tracing::warn!(
            "Failed to sync Caddy configuration after adding domain '{}' to app '{}': {}",
            domain,
            app_name,
            e
        );
        // Don't fail the operation if Caddy sync fails -- the DB write
        // already succeeded and a later sync (e.g. next deploy) will pick
        // it up.
    }

    Ok(())
}

/// Remove a custom top-level domain from `app_name`'s route and resync Caddy.
#[instrument(skip(pool, docker))]
pub async fn remove(pool: &Pool<Sqlite>, docker: &Docker, app_name: &str, domain: &str) -> Result_<()> {
    let domain = domain.trim();

    let mut app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| DomainError::AppNotFound(app_name.to_string()))?;

    let mut domains = app.custom_domains_list();
    let original_len = domains.len();
    domains.retain(|d| d != domain);
    if domains.len() == original_len {
        return Err(DomainError::DomainNotFound(
            app_name.to_string(),
            domain.to_string(),
        ));
    }

    app.custom_domains = if domains.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&domains).expect("Vec<String> always serializes"))
    };

    db::app::save(pool, &app).await?;
    info!("Removed custom domain '{}' from app '{}'", domain, app_name);

    if let Err(e) = caddy::sync_configuration(docker, pool).await {
        tracing::warn!(
            "Failed to sync Caddy configuration after removing domain '{}' from app '{}': {}",
            domain,
            app_name,
            e
        );
    }

    Ok(())
}

/// List the custom domains configured for `app_name`.
#[instrument(skip(pool))]
pub async fn list(pool: &Pool<Sqlite>, app_name: &str) -> Result_<Vec<String>> {
    let app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| DomainError::AppNotFound(app_name.to_string()))?;

    Ok(app.custom_domains_list())
}
