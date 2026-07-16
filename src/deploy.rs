//! Deploy engine: pull an image, replace an app's running container with it,
//! sync Caddy, and record the outcome in the `deploy` table.
//!
//! This is the single code path behind both the public GitHub deploy hook
//! (`POST /hooks/deploy`) and the admin-triggered redeploy
//! (`POST /apps/:name/deploy`).

use anyhow::{Context, Result};
use bollard::Docker;
use sqlx::{Pool, Sqlite};

use crate::commands::start::start_container;
use crate::models::{App, Deploy};
use crate::{caddy, db, docker};

/// Returns true when `provided` hashes to the app's stored deploy-token hash.
/// A missing or empty stored hash never authorizes anything.
pub fn verify_deploy_token(provided: &str, stored_hash: Option<&str>) -> bool {
    match stored_hash {
        Some(hash) if !hash.is_empty() => {
            crate::auth::constant_time_eq(&crate::auth::hash_token(provided), hash)
        }
        _ => false,
    }
}

/// True when `image` belongs to the app's own GHCR namespace derived from
/// `repo` (`"owner/name"`). Guards the public deploy hook: a per-app deploy
/// token lives as a GitHub Actions secret, so a leaked token must only be able
/// to (re)deploy *that app's own* images — not an arbitrary registry
/// reference that would run attacker code under the app's identity and volume.
///
/// GHCR image paths are always lowercase (GitHub enforces it), matching what
/// the committed workflow pushes: `ghcr.io/{owner}/{repo}:{tag|@digest}`.
/// A missing/empty `repo` cannot be validated and is therefore rejected.
pub fn image_matches_repo(image: &str, repo: Option<&str>) -> bool {
    let Some(repo) = repo.filter(|r| !r.is_empty()) else {
        return false;
    };
    let expected = format!("ghcr.io/{}", repo.to_lowercase());
    match image.strip_prefix(&expected) {
        // Exact match (no tag), or the next char begins a tag / digest. The
        // char check prevents `ghcr.io/o/app-evil` from matching `.../app`.
        Some("") => true,
        Some(rest) => rest.starts_with(':') || rest.starts_with('@'),
        None => false,
    }
}

/// Pull `image`, recreate the app container, sync Caddy, and record the
/// deploy. The old container keeps running until the new image has been
/// pulled successfully — a failed pull leaves the previous deploy untouched.
///
/// Always returns `Ok(Deploy)`; inspect `Deploy.status` ("succeeded" or
/// "failed") to determine the outcome. Only a genuine inability to look up
/// the app or persist the deploy record returns `Err`.
pub async fn deploy_app(
    pool: &Pool<Sqlite>,
    docker_conn: &Docker,
    app_name: &str,
    image: &str,
    git_sha: Option<&str>,
) -> Result<Deploy> {
    let app = db::app::get_by_name(pool, app_name)
        .await?
        .with_context(|| format!("unknown app '{app_name}'"))?;

    let mut record = Deploy::new(&app.id, image, git_sha);
    db::deploy::insert(pool, &record).await?;

    let result = do_deploy(pool, docker_conn, &app, image).await;
    match &result {
        Ok(()) => {
            db::deploy::set_status(pool, &record.id, "succeeded", None).await?;
            record.status = "succeeded".into();
        }
        Err(e) => {
            let message = format!("{e:#}");
            db::deploy::set_status(pool, &record.id, "failed", Some(&message)).await?;
            record.status = "failed".into();
            record.error = Some(message);
        }
    }

    Ok(record)
}

async fn do_deploy(
    pool: &Pool<Sqlite>,
    docker_conn: &Docker,
    app: &App,
    image: &str,
) -> Result<()> {
    let ghcr_token = db::system_config::get_ghcr_token(pool).await?;
    docker::pull(docker_conn, image, ghcr_token.as_deref())
        .await
        .context("failed to pull image")?;

    let exposed_port = docker::get_exposed_port(image)
        .await
        .context("failed to inspect image for EXPOSE port")?;

    // Point of no return: the pull succeeded, so it's now safe to replace the
    // running container. Unconditionally stop+remove first (rather than
    // relying on `docker::run`'s "already running" shortcut) because that
    // shortcut matches on container *name*, not image — it would otherwise
    // silently keep the old image running under a new image tag.
    docker::stop_and_remove_container(docker_conn, &app.name)
        .await
        .context("failed to remove existing container")?;

    start_container(pool, docker_conn, app, image)
        .await
        .context("failed to start new container")?;

    db::app::set_deployed(pool, &app.id, image, &exposed_port).await?;

    // Caddy sync failure must not fail the deploy: the container is up and
    // serving on the litehouse network either way, and an operator can
    // re-sync Caddy independently. Failing the deploy here would make a
    // perfectly good container replacement look like a broken deploy.
    if let Err(e) = caddy::sync_configuration(docker_conn, pool).await {
        tracing::warn!(
            "Caddy sync failed after deploying '{}' (container is up, routing may be stale): {e:#}",
            app.name
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_deploy_token_correct() {
        let hash = crate::auth::hash_token("secret");
        assert!(verify_deploy_token("secret", Some(&hash)));
    }

    #[test]
    fn verify_deploy_token_wrong() {
        let hash = crate::auth::hash_token("secret");
        assert!(!verify_deploy_token("wrong", Some(&hash)));
    }

    #[test]
    fn verify_deploy_token_no_hash() {
        assert!(!verify_deploy_token("anything", None));
    }

    #[test]
    fn verify_deploy_token_empty_hash() {
        assert!(!verify_deploy_token("anything", Some("")));
        assert!(!verify_deploy_token("", Some("")));
    }

    #[test]
    fn image_matches_repo_accepts_own_namespace() {
        let repo = Some("danbruder/hello");
        assert!(image_matches_repo("ghcr.io/danbruder/hello:latest", repo));
        assert!(image_matches_repo("ghcr.io/danbruder/hello:abc123sha", repo));
        assert!(image_matches_repo(
            "ghcr.io/danbruder/hello@sha256:deadbeef",
            repo
        ));
        assert!(image_matches_repo("ghcr.io/danbruder/hello", repo));
    }

    #[test]
    fn image_matches_repo_lowercases_repo() {
        // app.repo may be stored with the GitHub owner's original casing;
        // GHCR paths are lowercase, so the comparison must normalize.
        assert!(image_matches_repo(
            "ghcr.io/danbruder/hello:latest",
            Some("DanBruder/Hello")
        ));
    }

    #[test]
    fn image_matches_repo_rejects_foreign_and_prefix_confusion() {
        let repo = Some("danbruder/hello");
        // Different registry / owner / repo.
        assert!(!image_matches_repo("ghcr.io/attacker/evil:latest", repo));
        assert!(!image_matches_repo("docker.io/library/nginx:latest", repo));
        // Prefix-confusion: a repo that merely starts with ours.
        assert!(!image_matches_repo("ghcr.io/danbruder/hello-evil:latest", repo));
        assert!(!image_matches_repo("ghcr.io/danbruder/helloworld", repo));
        // Registry-host confusion.
        assert!(!image_matches_repo(
            "ghcr.io.attacker.com/danbruder/hello:latest",
            repo
        ));
    }

    #[test]
    fn image_matches_repo_rejects_missing_repo() {
        assert!(!image_matches_repo("ghcr.io/danbruder/hello:latest", None));
        assert!(!image_matches_repo("ghcr.io/danbruder/hello:latest", Some("")));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::db::test::get_test_pool;

    /// Full round trip against real Docker: create an app row, deploy a
    /// public multi-arch image that EXPOSEs a port, verify the container is
    /// running and the app record reflects the deploy, then deploy again to
    /// exercise the container-replacement path.
    #[tokio::test]
    async fn test_deploy_app_happy_path_and_redeploy() {
        let pool = get_test_pool().await;
        let docker_conn = docker::connect().await.expect("connect to docker");

        let app_name = "deploy-it-test-nginx";
        let container_name = format!("{}-container", app_name);

        // Clean up any leftovers from a previous failed run.
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", &container_name])
            .output();
        let _ = std::process::Command::new("docker")
            .args(["volume", "rm", "-f", &format!("litehouse-db-{}", app_name)])
            .output();

        let app = crate::models::App::new(app_name).unwrap();
        db::app::save(&pool, &app).await.unwrap();
        // Deploying needs the app's data volume to already exist (created by
        // `lh create` in production); replicate that here.
        crate::volume::create_app_volume(&docker_conn, &app.id)
            .await
            .expect("create app volume");

        let deploy = deploy_app(&pool, &docker_conn, app_name, "nginx:alpine", Some("abc123"))
            .await
            .expect("deploy_app should not hard-error");

        assert_eq!(deploy.status, "succeeded", "deploy error: {:?}", deploy.error);

        let updated = db::app::get_by_name(&pool, app_name).await.unwrap().unwrap();
        assert_eq!(updated.image.as_deref(), Some("nginx:alpine"));
        assert_eq!(updated.exposed_port.as_deref(), Some("80"));
        assert!(updated.is_running());

        let state = std::process::Command::new("docker")
            .args(["ps", "--filter", &format!("name={container_name}"), "--format", "{{.State}}"])
            .output()
            .unwrap();
        let state = String::from_utf8_lossy(&state.stdout);
        assert!(state.trim() == "running", "container should be running, got: {state}");

        // Redeploy with the same image to exercise the replace-existing-container path.
        let second = deploy_app(&pool, &docker_conn, app_name, "nginx:alpine", Some("def456"))
            .await
            .expect("redeploy should not hard-error");
        assert_eq!(second.status, "succeeded", "redeploy error: {:?}", second.error);

        let deploys = db::deploy::list_for_app(&pool, &app.id, 10).await.unwrap();
        assert_eq!(deploys.len(), 2);
        assert_eq!(deploys[0].git_sha.as_deref(), Some("def456"));

        // Cleanup.
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", &container_name])
            .output();
        let _ = std::process::Command::new("docker")
            .args(["volume", "rm", "-f", &format!("litehouse-db-{}", app.id)])
            .output();
    }
}
