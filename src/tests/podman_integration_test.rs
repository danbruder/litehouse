use anyhow::Result;
use std::fs;

#[tokio::test]
async fn podman_build_run_remove_smoke() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let dockerfile_path = tmp.path().join("Dockerfile");
    fs::write(
        &dockerfile_path,
        "FROM docker.io/library/alpine:3.20\nCMD [\"echo\",\"ok\"]\n",
    )?;

    let tag = format!("bindrop-int:{}", uuid::Uuid::new_v4());

    // build
    crate::podman::build(tmp.path().to_str().unwrap(), &tag).await?;

    // run (uses PODMAN_SSH_SOCK if set, otherwise default in code)
    crate::podman::run("bindrop-int", &tag).await?;

    // remove image (best-effort)
    let _ = crate::podman::remove(&tag).await;

    Ok(())
}
