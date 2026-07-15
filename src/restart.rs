use chrono::Timelike;

use bollard::Docker;
use sqlx::{Pool, Sqlite};
use tracing::instrument;

use crate::commands::server::{try_lock_app, AppLocks};
use crate::commands::start::start_container;
use crate::db;
use crate::docker;
use crate::models::App;

/// Env var key an app can set (via `lh env set`) to exclude itself from the
/// nightly restart pass.
pub const SKIP_ENV_VAR: &str = "LITEHOUSE_SKIP_NIGHTLY_RESTART";

#[derive(Debug, Default)]
pub struct RestartReport {
    pub restarted: Vec<String>,
    pub skipped: Vec<(String, String)>,
    pub failed: Vec<(String, String)>,
}

#[derive(Debug, PartialEq)]
pub enum RestartOutcome {
    Restarted,
    Skipped(&'static str),
}

#[derive(Debug, thiserror::Error)]
pub enum RestartError {
    #[error("failed to load env vars: {0}")]
    EnvVarLoadFailed(String),
    #[error("failed to stop container: {0}")]
    StopFailed(String),
    #[error("failed to start container: {0}")]
    StartFailed(String),
    #[error("app has no deployed image")]
    NoImage,
}

/// True if, given the current Eastern time and the Eastern date
/// (`YYYY-MM-DD`) the nightly restart last completed (`None` if it has
/// never run), tonight's pass should run now. Pure and independent of
/// Docker/DB so it's directly unit-testable; the scheduler loop in
/// `commands::server::execute` calls this once per hourly tick.
pub fn should_run_now(now_eastern: chrono::DateTime<chrono_tz::Tz>, last_run_date: Option<&str>) -> bool {
    if now_eastern.hour() != 3 {
        return false;
    }
    let today = now_eastern.format("%Y-%m-%d").to_string();
    last_run_date != Some(today.as_str())
}

/// Restart a single app if it's currently running, not locked by another
/// operation, and not opted out. Never blocks waiting for the app's lock —
/// if a deploy or manual action currently holds it, this returns
/// `Skipped` immediately rather than delaying or contending with it.
#[instrument(skip(pool, docker, app_locks, app), fields(app = %app.name))]
pub async fn restart_one_app(
    pool: &Pool<Sqlite>,
    docker: &Docker,
    app_locks: &AppLocks,
    app: &App,
) -> Result<RestartOutcome, RestartError> {
    // Opt-out is checked before the live Docker state so that an app which
    // has excluded itself never pays the cost of (and can't race on) a
    // Docker inspect call.
    let env_vars = db::env_var::get_by_app(pool, &app.id)
        .await
        .map_err(|e| RestartError::EnvVarLoadFailed(e.to_string()))?;
    if env_vars
        .iter()
        .any(|e| e.key == SKIP_ENV_VAR && e.value.eq_ignore_ascii_case("true"))
    {
        return Ok(RestartOutcome::Skipped(
            "opted out via LITEHOUSE_SKIP_NIGHTLY_RESTART",
        ));
    }

    match docker::live_state(&app.name).await {
        Ok(crate::models::AppState::Running) => {}
        Ok(_) => return Ok(RestartOutcome::Skipped("not running")),
        Err(e) => {
            tracing::warn!(app = %app.name, "nightly restart: failed to check live state: {e:#}");
            return Ok(RestartOutcome::Skipped("live state check failed"));
        }
    }

    let Some(_guard) = try_lock_app(app_locks, &app.name) else {
        return Ok(RestartOutcome::Skipped("app lock held by another operation"));
    };

    // Re-check now that we hold the lock — it may have changed between the
    // check above and acquiring the lock (e.g. a deploy just replaced the
    // container).
    match docker::live_state(&app.name).await {
        Ok(crate::models::AppState::Running) => {}
        Ok(_) => return Ok(RestartOutcome::Skipped("no longer running")),
        Err(e) => {
            tracing::warn!(app = %app.name, "nightly restart: failed to re-check live state after acquiring lock: {e:#}");
            return Ok(RestartOutcome::Skipped("live state re-check failed"));
        }
    }

    let image = app.image.as_deref().ok_or(RestartError::NoImage)?;

    docker::stop(app)
        .await
        .map_err(|e| RestartError::StopFailed(e.to_string()))?;

    start_container(pool, docker, app, image)
        .await
        .map_err(|e| RestartError::StartFailed(e.to_string()))?;

    let mut updated_app = app.clone();
    updated_app.state = crate::models::AppState::Running;
    db::app::save(pool, &updated_app)
        .await
        .map_err(|e| RestartError::StartFailed(format!("restarted but failed to save state: {e}")))?;

    Ok(RestartOutcome::Restarted)
}

/// Restart every currently-running app, skipping ones that are locked or
/// opted out. Called once per completed scheduling tick (see
/// `commands::server::execute`).
#[instrument(skip(pool, docker, app_locks))]
pub async fn restart_running_apps(
    pool: &Pool<Sqlite>,
    docker: &Docker,
    app_locks: &AppLocks,
) -> RestartReport {
    let mut report = RestartReport::default();

    let apps = match db::app::get_all(pool).await {
        Ok(apps) => apps,
        Err(e) => {
            tracing::error!("nightly restart: failed to list apps: {e:#}");
            return report;
        }
    };

    for app in apps {
        match restart_one_app(pool, docker, app_locks, &app).await {
            Ok(RestartOutcome::Restarted) => report.restarted.push(app.name.clone()),
            Ok(RestartOutcome::Skipped(reason)) => {
                report.skipped.push((app.name.clone(), reason.to_string()))
            }
            Err(e) => report.failed.push((app.name.clone(), e.to_string())),
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn eastern(y: i32, m: u32, d: u32, h: u32) -> chrono::DateTime<chrono_tz::Tz> {
        chrono_tz::America::New_York
            .with_ymd_and_hms(y, m, d, h, 0, 0)
            .unwrap()
    }

    #[test]
    fn should_run_at_3am_when_never_run_before() {
        assert!(should_run_now(eastern(2026, 7, 15, 3), None));
    }

    #[test]
    fn should_not_run_outside_the_3am_hour() {
        assert!(!should_run_now(eastern(2026, 7, 15, 2), None));
        assert!(!should_run_now(eastern(2026, 7, 15, 4), None));
        assert!(!should_run_now(eastern(2026, 7, 15, 0), None));
    }

    #[test]
    fn should_not_run_twice_in_the_same_eastern_day() {
        assert!(!should_run_now(eastern(2026, 7, 15, 3), Some("2026-07-15")));
    }

    #[test]
    fn should_run_again_the_next_day() {
        assert!(should_run_now(eastern(2026, 7, 16, 3), Some("2026-07-15")));
    }

    use crate::db::test::get_test_pool;
    use crate::models::EnvVar;

    async fn make_app(pool: &sqlx::Pool<sqlx::Sqlite>, name: &str) -> App {
        let app = App::new(name).unwrap();
        crate::db::app::save(pool, &app).await.unwrap();
        app
    }

    #[tokio::test]
    async fn restart_one_app_skips_when_not_running() {
        let pool = get_test_pool().await;
        let docker = crate::docker::connect().await.unwrap();
        let locks: AppLocks = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        // No container exists for this app name, so live_state reports Stopped.
        let app = make_app(&pool, "restart-test-not-running").await;

        let outcome = restart_one_app(&pool, &docker, &locks, &app).await.unwrap();
        assert_eq!(outcome, RestartOutcome::Skipped("not running"));
    }

    #[tokio::test]
    async fn restart_one_app_skips_when_opted_out() {
        let pool = get_test_pool().await;
        let docker = crate::docker::connect().await.unwrap();
        let locks: AppLocks = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let app = make_app(&pool, "restart-test-opted-out").await;
        crate::db::env_var::save(
            &pool,
            &EnvVar::new(&app.id, SKIP_ENV_VAR, "true"),
        )
        .await
        .unwrap();

        // Opt-out is checked before the live Docker state, so no real
        // container is needed to exercise this path.
        let outcome = restart_one_app(&pool, &docker, &locks, &app).await.unwrap();
        assert_eq!(outcome, RestartOutcome::Skipped("opted out via LITEHOUSE_SKIP_NIGHTLY_RESTART"));
    }

    /// Requires Docker. Run with:
    ///   DOCKER_API_VERSION=1.42 cargo test restart_one_app_restarts_a_running_container -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn restart_one_app_restarts_a_running_container() {
        let pool = get_test_pool().await;
        let docker = crate::docker::connect().await.unwrap();
        let locks: AppLocks = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

        let app_name = "restart-test-live";
        let container_name = format!("{}-container", app_name);
        let image_tag = "redis:7.4-alpine";

        // Clean up any leftover container from a previous failed/interrupted run.
        let _ = crate::docker::test_helpers::cleanup_container(&container_name);

        crate::docker::run(app_name, image_tag, vec![], vec![]).await.unwrap();
        assert!(crate::docker::test_helpers::is_container_started(&container_name).unwrap());

        let mut app = make_app(&pool, app_name).await;
        app.image = Some(image_tag.to_string());
        crate::db::app::save(&pool, &app).await.unwrap();

        let outcome = restart_one_app(&pool, &docker, &locks, &app).await.unwrap();
        assert_eq!(outcome, RestartOutcome::Restarted);

        let state = crate::docker::test_helpers::get_container_state(&container_name).unwrap();
        assert_eq!(state, "running");

        let reloaded = crate::db::app::get_by_name(&pool, app_name).await.unwrap().unwrap();
        assert_eq!(reloaded.state, crate::models::AppState::Running);

        crate::docker::test_helpers::cleanup_container(&container_name).unwrap();
    }
}
