use anyhow::Result;
use sqlx::{Pool, Sqlite};
use tracing::instrument;

use crate::db::DatabaseError;
use crate::models::{
    utc_datetime, WebhookConfig, WebhookDelivery, WebhookDeliveryStatus, WebhookStatus,
};

// ===== WebhookConfig Operations =====

#[instrument(skip(pool))]
pub async fn get_webhook_config_by_app(
    pool: &Pool<Sqlite>,
    app_id: &str,
) -> Result<Option<WebhookConfig>, DatabaseError> {
    let result = sqlx::query!(
        r#"
        SELECT id, app_id, enabled, secret, auto_deploy, github_webhook_id,
               status, error_message, created_at, updated_at
        FROM webhook_config
        WHERE app_id = ?
        "#,
        app_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(result.map(|row| WebhookConfig {
        id: row.id,
        app_id: row.app_id,
        enabled: row.enabled,
        secret: row.secret,
        auto_deploy: row.auto_deploy,
        github_webhook_id: row.github_webhook_id,
        status: WebhookStatus::from_str(&row.status),
        error_message: row.error_message,
        created_at: row.created_at.into(),
        updated_at: row.updated_at.into(),
    }))
}

#[instrument(skip(pool, config))]
pub async fn save_webhook_config(
    pool: &Pool<Sqlite>,
    config: &WebhookConfig,
) -> Result<(), DatabaseError> {
    let enabled = config.enabled;
    let auto_deploy = config.auto_deploy;
    let status = config.status.as_str();

    sqlx::query!(
        r#"
        INSERT INTO webhook_config (id, app_id, enabled, secret, auto_deploy, github_webhook_id, status, error_message, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(app_id) DO UPDATE SET
            enabled = excluded.enabled,
            secret = excluded.secret,
            auto_deploy = excluded.auto_deploy,
            github_webhook_id = excluded.github_webhook_id,
            status = excluded.status,
            error_message = excluded.error_message,
            updated_at = excluded.updated_at
        "#,
        config.id,
        config.app_id,
        enabled,
        config.secret,
        auto_deploy,
        config.github_webhook_id,
        status,
        config.error_message,
        config.created_at,
        config.updated_at
    )
    .execute(pool)
    .await?;

    Ok(())
}

#[instrument(skip(pool))]
pub async fn update_webhook_status(
    pool: &Pool<Sqlite>,
    app_id: &str,
    status: WebhookStatus,
    github_webhook_id: Option<i64>,
    error: Option<String>,
) -> Result<(), DatabaseError> {
    let status_str = status.as_str();
    let updated_at = utc_datetime::now();

    sqlx::query!(
        r#"
        UPDATE webhook_config
        SET status = ?, github_webhook_id = ?, error_message = ?, updated_at = ?
        WHERE app_id = ?
        "#,
        status_str,
        github_webhook_id,
        error,
        updated_at,
        app_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

#[instrument(skip(pool))]
pub async fn delete_webhook_config_by_app(
    pool: &Pool<Sqlite>,
    app_id: &str,
) -> Result<(), DatabaseError> {
    sqlx::query!(
        r#"
        DELETE FROM webhook_config
        WHERE app_id = ?
        "#,
        app_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

// ===== WebhookDelivery Operations =====

#[instrument(skip(pool, delivery))]
pub async fn save_webhook_delivery(
    pool: &Pool<Sqlite>,
    delivery: &WebhookDelivery,
) -> Result<(), DatabaseError> {
    let status = delivery.status.as_str();

    sqlx::query!(
        r#"
        INSERT INTO webhook_delivery (id, app_id, github_delivery_id, github_event, repository_url, ref, commit_sha, status, error_message, build_id, payload_snippet, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
        delivery.id,
        delivery.app_id,
        delivery.github_delivery_id,
        delivery.github_event,
        delivery.repository_url,
        delivery.ref_,
        delivery.commit_sha,
        status,
        delivery.error_message,
        delivery.build_id,
        delivery.payload_snippet,
        delivery.created_at
    )
    .execute(pool)
    .await?;

    Ok(())
}

#[instrument(skip(pool))]
pub async fn get_webhook_deliveries_by_app(
    pool: &Pool<Sqlite>,
    app_id: &str,
    limit: i64,
) -> Result<Vec<WebhookDelivery>, DatabaseError> {
    let rows = sqlx::query!(
        r#"
        SELECT id, app_id, github_delivery_id, github_event, repository_url, ref, commit_sha, status, error_message, build_id, payload_snippet, created_at
        FROM webhook_delivery
        WHERE app_id = ?
        ORDER BY created_at DESC
        LIMIT ?
        "#,
        app_id,
        limit
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| WebhookDelivery {
            id: row.id,
            app_id: row.app_id,
            github_delivery_id: row.github_delivery_id,
            github_event: row.github_event,
            repository_url: row.repository_url,
            ref_: row.r#ref,
            commit_sha: row.commit_sha,
            status: WebhookDeliveryStatus::from_str(&row.status),
            error_message: row.error_message,
            build_id: row.build_id,
            payload_snippet: row.payload_snippet,
            created_at: row.created_at.into(),
        })
        .collect())
}
