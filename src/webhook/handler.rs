use anyhow::Result;
use bollard::Docker;
use sqlx::{Pool, Sqlite};
use std::sync::Arc;
use tracing::{debug, error, info, instrument, warn};

use crate::commands::{build, start};
use crate::db;
use crate::message_bus::{Message, MessageBus};
use crate::models::{
    App, GitHubPushPayload, WebhookDelivery, WebhookDeliveryStatus,
};
use crate::webhook::verification;

#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    #[error("Signature verification failed: {0}")]
    SignatureError(#[from] verification::VerificationError),
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
    #[error("Invalid payload: {0}")]
    InvalidPayload(String),
    #[error("App not found for repository: {0}")]
    AppNotFound(String),
    #[error("Build error: {0}")]
    BuildError(String),
}

/// Handle incoming GitHub webhook
#[instrument(skip(pool, docker, message_bus, payload))]
pub async fn handle_github_webhook(
    pool: &Pool<Sqlite>,
    docker: &Docker,
    message_bus: Arc<MessageBus>,
    github_token: Option<String>,
    event_type: String,
    delivery_id: Option<String>,
    signature: String,
    payload: &[u8],
) -> Result<WebhookDelivery, WebhookError> {
    info!(
        "Received GitHub webhook: event={}, delivery_id={:?}",
        event_type, delivery_id
    );

    // Only process push events for now
    if event_type != "push" {
        debug!("Ignoring non-push event: {}", event_type);
        let delivery = WebhookDelivery::new(
            None,
            delivery_id,
            event_type,
            "unknown".to_string(),
            WebhookDeliveryStatus::IgnoredEvent,
        );
        db::webhook::save_webhook_delivery(pool, &delivery).await?;
        return Ok(delivery);
    }

    // Parse payload
    let push_payload: GitHubPushPayload = serde_json::from_slice(payload)
        .map_err(|e| WebhookError::InvalidPayload(e.to_string()))?;

    // Find app by repository URL
    let app = match find_app_by_repo_url(pool, &push_payload.repository).await? {
        Some(app) => app,
        None => {
            warn!(
                "No app found for repository: {}",
                push_payload.repository.clone_url
            );
            let delivery = WebhookDelivery::new(
                None,
                delivery_id,
                event_type,
                push_payload.repository.clone_url.clone(),
                WebhookDeliveryStatus::AppNotFound,
            );
            db::webhook::save_webhook_delivery(pool, &delivery).await?;
            return Ok(delivery);
        }
    };

    // Get webhook configuration
    let webhook_config = match db::webhook::get_webhook_config_by_app(pool, &app.id).await? {
        Some(config) => config,
        None => {
            warn!("No webhook configuration found for app: {}", app.name);
            let mut delivery = WebhookDelivery::new(
                Some(app.id.clone()),
                delivery_id,
                event_type,
                push_payload.repository.clone_url.clone(),
                WebhookDeliveryStatus::AppNotFound,
            );
            delivery.error_message = Some("No webhook configuration".to_string());
            db::webhook::save_webhook_delivery(pool, &delivery).await?;
            return Ok(delivery);
        }
    };

    // Verify signature
    if let Err(e) = verification::verify_github_signature(payload, &signature, &webhook_config.secret)
    {
        warn!("Webhook signature verification failed for app '{}': {}", app.name, e);
        let mut delivery = WebhookDelivery::new(
            Some(app.id.clone()),
            delivery_id,
            event_type,
            push_payload.repository.clone_url.clone(),
            WebhookDeliveryStatus::SignatureInvalid,
        );
        delivery.error_message = Some(e.to_string());
        db::webhook::save_webhook_delivery(pool, &delivery).await?;
        return Err(WebhookError::SignatureError(e));
    }

    debug!("Webhook signature verified for app '{}'", app.name);

    // Extract commit info
    let commit_sha = push_payload
        .head_commit
        .as_ref()
        .map(|c| c.id.clone());
    let ref_ = Some(push_payload.ref_.clone());

    // Create delivery record with matched status
    let mut delivery = WebhookDelivery::new(
        Some(app.id.clone()),
        delivery_id.clone(),
        event_type.clone(),
        push_payload.repository.clone_url.clone(),
        WebhookDeliveryStatus::Matched,
    );
    delivery.commit_sha = commit_sha;
    delivery.ref_ = ref_;

    // Store a snippet of the payload for debugging
    let payload_str = String::from_utf8_lossy(payload);
    delivery.payload_snippet = Some(
        payload_str
            .chars()
            .take(500)
            .collect::<String>()
    );

    // Trigger build
    info!("Triggering build for app '{}' from webhook", app.name);
    match build::execute(
        pool,
        &app.name,
        github_token.as_deref(),
        message_bus.clone(),
        false, // force=false for webhooks
    )
    .await
    {
        Ok(build) => {
            info!(
                "Build triggered successfully for app '{}', build_id: {}",
                app.name, build.id
            );
            delivery.status = WebhookDeliveryStatus::BuildTriggered;
            delivery.build_id = Some(build.id.clone());

            // Schedule auto-deploy if enabled
            if webhook_config.auto_deploy {
                schedule_auto_deploy(
                    pool.clone(),
                    docker.clone(),
                    message_bus.clone(),
                    app.name.clone(),
                    build.id,
                );
            }

            // Publish webhook received event
            message_bus.publish(Message::WebhookReceived {
                app_name: app.name.clone(),
                event_type,
                status: "build_triggered".to_string(),
                delivery_id,
            });
        }
        Err(e) => {
            error!("Failed to trigger build for app '{}': {}", app.name, e);
            delivery.status = WebhookDeliveryStatus::BuildFailed;
            delivery.error_message = Some(e.to_string());

            // Publish webhook received event
            message_bus.publish(Message::WebhookReceived {
                app_name: app.name.clone(),
                event_type,
                status: "build_failed".to_string(),
                delivery_id,
            });
        }
    }

    // Save delivery record
    db::webhook::save_webhook_delivery(pool, &delivery).await?;

    Ok(delivery)
}

/// Find an app by matching its remote repository URL
#[instrument(skip(pool))]
async fn find_app_by_repo_url(
    pool: &Pool<Sqlite>,
    repo: &crate::models::Repository,
) -> Result<Option<App>, crate::db::DatabaseError> {
    let apps = db::app::get_all(pool).await?;

    for app in apps {
        if let Ok(remote) = db::remote::get_by_app(pool, &app.id).await {
            if normalize_url(&remote.remote) == normalize_url(&repo.clone_url)
                || normalize_url(&remote.remote) == normalize_url(&repo.ssh_url)
            {
                return Ok(Some(app));
            }
        }
    }

    Ok(None)
}

/// Normalize a Git repository URL for comparison
fn normalize_url(url: &str) -> String {
    url.trim_end_matches('/')
        .trim_end_matches(".git")
        .to_lowercase()
}

/// Schedule auto-deploy task that waits for build success
fn schedule_auto_deploy(
    pool: Pool<Sqlite>,
    docker: Docker,
    message_bus: Arc<MessageBus>,
    app_name: String,
    build_id: String,
) {
    tokio::spawn(async move {
        let mut rx = message_bus.subscribe();
        let timeout = tokio::time::sleep(tokio::time::Duration::from_secs(600)); // 10 minute timeout
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                msg_result = rx.recv() => {
                    match msg_result {
                        Ok(msg) => {
                            if let Message::BuildStatus {
                                build_id: msg_build_id,
                                status,
                                ..
                            } = msg
                            {
                                if msg_build_id == build_id {
                                    if status == "success" {
                                        info!("Auto-deploying '{}' after successful build", app_name);

                                        if let Err(e) = start::execute(&pool, &docker, &app_name).await {
                                            error!("Auto-deploy failed for '{}': {}", app_name, e);
                                        } else {
                                            info!("Auto-deploy completed successfully for '{}'", app_name);
                                        }
                                        break;
                                    } else if status == "failed" {
                                        warn!("Build failed for '{}', skipping auto-deploy", app_name);
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("Error receiving message for auto-deploy: {}", e);
                            break;
                        }
                    }
                }
                _ = &mut timeout => {
                    warn!("Auto-deploy timeout for app '{}' after 10 minutes", app_name);
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_url() {
        assert_eq!(
            normalize_url("https://github.com/user/repo.git"),
            normalize_url("https://github.com/user/repo")
        );
        assert_eq!(
            normalize_url("https://github.com/user/repo/"),
            normalize_url("https://github.com/user/repo")
        );
        assert_eq!(
            normalize_url("HTTPS://GITHUB.COM/User/Repo"),
            normalize_url("https://github.com/user/repo")
        );
    }
}
