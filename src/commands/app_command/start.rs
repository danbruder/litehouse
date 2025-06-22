use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::db;
use crate::podman;
use crate::providers::{Handle, Provider};

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum StartError {
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("App already running: {0}")]
    AppAlreadyRunning(String),
    #[error("App not deployed: {0}")]
    AppNotDeployed(String),
    #[error("Failed to start app: {0}")]
    AppStartFailed(String),
    #[error("App log broken: {0}")]
    AppLogBroken(String),
    #[error("Invalid binary path: {0}")]
    InvalidBinaryPath(String),
    #[error("Database error")]
    DatabaseError(String),
    #[error("Config error: {0}")]
    ConfigError(#[from] crate::config::ConfigError),
}

type Result<T> = anyhow::Result<T, StartError>;

impl From<crate::db::DatabaseError> for StartError {
    fn from(err: crate::db::DatabaseError) -> Self {
        StartError::DatabaseError(err.to_string())
    }
}

/// Start an app using the supervisor
#[instrument(skip(pool))]
pub async fn execute(pool: &Pool<Sqlite>, app_name: &str) -> Result<()> {
    // VALIDATION
    let app = db::apps::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| StartError::AppNotFound(app_name.to_string()))?;

    if !app.is_built() {
        return Err(StartError::AppNotDeployed(app.name.clone()));
    }

    // Start
    let (app, change) = app.started();
    db::apps::save(pool, &app).await?;
    db::app_state_change::save(pool, &change).await?;

    podman::run(&app).await?;

    // Check it is running, then update running

    // Update app with process ID
    let (app, change) = app.running();
    db::apps::save(pool, &app).await?;
    db::app_state_change::save(pool, &change).await?;

    info!("Started app '{}' with PID {}", app.name, process_id);

    Ok(handle)
}

#[cfg(test)]
mod test {
    use crate::db::apps;
    use crate::models::App;
    use crate::providers::test::TestProvider;

    #[tokio::test]
    async fn test_starting_non_existant_app() {
        let pool = crate::db::test::get_test_pool().await;
        let got = super::execute(&pool, "non_existant_app", TestProvider)
            .await
            .unwrap_err();
        let want = super::StartError::AppNotFound("non_existant_app".to_string());

        assert_eq!(got, want);
    }

    #[tokio::test]
    async fn test_starting_not_deployed_app() {
        let pool = crate::db::test::get_test_pool().await;
        let app_name = "app";
        let app = App::new(app_name).unwrap();
        apps::save(&pool, &app).await.unwrap();

        let got = super::execute(&pool, app_name, TestProvider)
            .await
            .unwrap_err();
        let want = super::StartError::AppNotDeployed(app_name.to_string());

        assert_eq!(got, want);
    }

    #[tokio::test]
    async fn test_start_happy_path() {
        let pool = crate::db::test::get_test_pool().await;
        let app_name = "app";
        let app = App::new(app_name).unwrap().deployed("".into(), "".into());
        apps::save(&pool, &app).await.unwrap();

        let got = super::execute(&pool, app_name, TestProvider).await.unwrap();
        let want = true;

        assert_eq!(got, want);
    }
}
