//! Renders the GitHub Actions workflow that litehouse commits into a user's
//! repo on `lh create`. The workflow builds and pushes a container image to
//! GHCR, then calls the litehouse deploy hook with the freshly built image.
//!
//! The image is built for the litehouse server's platform (`linux/amd64` or
//! `linux/arm64`, as reported by `GET /api/server/info`). GitHub's standard
//! runners are amd64, so an arm64 image is built under QEMU emulation — slower,
//! but it works on every repo (public or private) with no runner setup.
//!
//! Placeholders (`__OWNER__`, `__REPO__`, `__APP__`, `__HOOK_URL__`,
//! `__PLATFORM__`, `__EMULATION_STEPS__`) are substituted via
//! plain string replacement rather than `format!`, since the template is
//! full of literal `${{ ... }}` GitHub Actions expression syntax that would
//! otherwise have to be escaped as `{{{{ ... }}}}`.

const TEMPLATE: &str = r#"name: __NAME__
on:
  push:
    branches: [main, master]
__PATHS__jobs:
  deploy:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      packages: write
    steps:
      - uses: actions/checkout@v4
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
__EMULATION_STEPS__      - uses: docker/build-push-action@v6
        with:
          context: __CONTEXT__
          platforms: __PLATFORM__
          push: true
          tags: |
            __IMAGE__:latest
            __IMAGE__:${{ github.sha }}
      - name: notify litehouse
        run: |
          curl -fsS -X POST "__HOOK_URL__" \
            -H "Authorization: Bearer ${{ secrets.__SECRET__ }}" \
            -H "Content-Type: application/json" \
            -d "{\"app\":\"__APP__\",\"image\":\"__IMAGE__:${{ github.sha }}\",\"sha\":\"${{ github.sha }}\"}"
"#;

/// Extra steps for building a non-amd64 image on GitHub's amd64 runners.
const EMULATION_STEPS: &str = r#"      - uses: docker/setup-qemu-action@v3
      - uses: docker/setup-buildx-action@v3
"#;

/// Platform used when the server doesn't report one (servers that predate
/// `platform` in `/api/server/info` were only ever released for amd64).
pub const DEFAULT_PLATFORM: &str = "linux/amd64";

/// Render the litehouse deploy workflow for `owner/app`, pointed at
/// `hook_url`. GHCR image paths must be lowercase (GitHub requires it), so
/// `owner` and `app` are lowercased wherever they appear in an image
/// reference or the hook payload's `app` field.
/// `owner`/`repo` name the GHCR image (GHCR paths follow the repo and must be
/// lowercase); `app` is the litehouse app name used in the deploy-hook payload
/// — they are NOT the same thing (repo "litehouse-hello" may back app "hello").
/// `platform` is the server's Docker platform, e.g. "linux/arm64".
pub fn render_deploy_workflow(
    owner: &str,
    repo: &str,
    app: &str,
    hook_url: &str,
    platform: &str,
) -> String {
    render_deploy_workflow_for(&DeployLayout::new(owner, repo, app, None), app, hook_url, platform)
}

/// Where an app's deploy wiring lives in its repo. An app built from the repo
/// root (the default) uses the classic single-app names; an app built from a
/// subdirectory (`lh create --context <dir>`) gets names of its own, so one
/// repo can deploy several apps side by side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployLayout {
    /// Docker build context, relative to the repo root.
    pub context: String,
    /// Repo path of the workflow file.
    pub workflow_path: String,
    /// GitHub Actions secret holding the app's deploy token.
    pub secret_name: String,
    /// GHCR image (lowercase, no tag).
    pub image: String,
    /// Only rebuild when these paths change (`None`: every push).
    pub paths: Option<Vec<String>>,
}

impl DeployLayout {
    pub fn new(owner: &str, repo: &str, app: &str, context: Option<&str>) -> Self {
        let owner = owner.to_lowercase();
        match context.map(|c| c.trim_matches('/')).filter(|c| !c.is_empty() && *c != ".") {
            None => DeployLayout {
                context: ".".into(),
                workflow_path: ".github/workflows/litehouse-deploy.yml".into(),
                secret_name: "LITEHOUSE_DEPLOY_TOKEN".into(),
                image: format!("ghcr.io/{}/{}", owner, repo.to_lowercase()),
                paths: None,
            },
            Some(dir) => {
                let workflow_path = format!(".github/workflows/litehouse-deploy-{}.yml", app);
                DeployLayout {
                    context: dir.to_string(),
                    secret_name: format!(
                        "LITEHOUSE_DEPLOY_TOKEN_{}",
                        app.to_uppercase().replace('-', "_")
                    ),
                    image: format!("ghcr.io/{}/{}", owner, app.to_lowercase()),
                    paths: Some(vec![format!("{}/**", dir), workflow_path.clone()]),
                    workflow_path,
                }
            }
        }
    }
}

/// Render the deploy workflow for `app` with an explicit layout.
pub fn render_deploy_workflow_for(
    layout: &DeployLayout,
    app: &str,
    hook_url: &str,
    platform: &str,
) -> String {
    let emulation = if platform == DEFAULT_PLATFORM { "" } else { EMULATION_STEPS };
    let (name, paths) = match &layout.paths {
        None => ("litehouse deploy".to_string(), String::new()),
        Some(ps) => (
            format!("litehouse deploy {}", app),
            format!(
                "    paths:\n{}",
                ps.iter().map(|p| format!("      - \"{}\"\n", p)).collect::<String>()
            ),
        ),
    };

    TEMPLATE
        .replace("__NAME__", &name)
        .replace("__PATHS__", &paths)
        .replace("__EMULATION_STEPS__", emulation)
        .replace("__PLATFORM__", platform)
        .replace("__CONTEXT__", &layout.context)
        .replace("__IMAGE__", &layout.image)
        .replace("__SECRET__", &layout.secret_name)
        .replace("__APP__", app)
        .replace("__HOOK_URL__", hook_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_renders_owner_app_and_hook() {
        let yml = render_deploy_workflow(
            "danbruder",
            "litehouse-hello",
            "hello",
            "https://admin.s.danbruder.com/api/hooks/deploy",
            DEFAULT_PLATFORM,
        );
        assert!(yml.contains("ghcr.io/danbruder/litehouse-hello:${{ github.sha }}"));
        // The hook payload names the litehouse APP, not the repo.
        assert!(yml.contains(r#"\"app\":\"hello\""#));
        assert!(!yml.contains(r#"\"app\":\"litehouse-hello\""#));
        assert!(yml.contains("https://admin.s.danbruder.com/api/hooks/deploy"));
        assert!(yml.contains("secrets.LITEHOUSE_DEPLOY_TOKEN"));
        assert!(!yml.contains('\t'));
    }

    #[test]
    fn workflow_lowercases_uppercase_owner_and_app_in_image_refs() {
        let yml = render_deploy_workflow(
            "DanBruder",
            "Hello",
            "myapp",
            "https://example.com/hooks/deploy",
            DEFAULT_PLATFORM,
        );
        assert!(yml.contains("ghcr.io/danbruder/hello:latest"));
        assert!(yml.contains("ghcr.io/danbruder/hello:${{ github.sha }}"));
        assert!(!yml.contains("ghcr.io/DanBruder"));
        assert!(!yml.contains("ghcr.io/danbruder/Hello"));
    }

    #[test]
    fn subdirectory_app_gets_its_own_workflow_secret_image_and_path_filter() {
        let layout = DeployLayout::new("DanBruder", "piper", "piper-media", Some("livekit/"));
        assert_eq!(layout.workflow_path, ".github/workflows/litehouse-deploy-piper-media.yml");
        assert_eq!(layout.secret_name, "LITEHOUSE_DEPLOY_TOKEN_PIPER_MEDIA");
        assert_eq!(layout.image, "ghcr.io/danbruder/piper-media");

        let yml = render_deploy_workflow_for(&layout, "piper-media", "https://e.com/hooks/deploy", DEFAULT_PLATFORM);
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yml).expect("valid yaml");
        let paths = parsed["on"]["push"]["paths"].as_sequence().expect("paths filter");
        assert_eq!(paths[0].as_str(), Some("livekit/**"));
        assert!(yml.contains("context: livekit\n"));
        assert!(yml.contains("secrets.LITEHOUSE_DEPLOY_TOKEN_PIPER_MEDIA"));
        assert!(yml.contains(r#"\"image\":\"ghcr.io/danbruder/piper-media:${{ github.sha }}\""#));
        assert!(!yml.contains("__"));
    }

    #[test]
    fn root_context_keeps_the_classic_single_app_layout() {
        for ctx in [None, Some("."), Some("")] {
            let layout = DeployLayout::new("o", "Repo", "app", ctx);
            assert_eq!(layout.workflow_path, ".github/workflows/litehouse-deploy.yml");
            assert_eq!(layout.secret_name, "LITEHOUSE_DEPLOY_TOKEN");
            assert_eq!(layout.image, "ghcr.io/o/repo");
            assert_eq!(layout.paths, None);
        }
        let yml = render_deploy_workflow("o", "r", "a", "https://e.com/hooks/deploy", DEFAULT_PLATFORM);
        assert!(yml.starts_with("name: litehouse deploy\non:\n  push:\n    branches: [main, master]\njobs:"));
    }

    #[test]
    fn workflow_contains_no_tabs_and_is_valid_yaml() {
        let yml =
            render_deploy_workflow("owner", "repo", "app", "https://example.com/hooks/deploy", DEFAULT_PLATFORM);
        assert!(!yml.contains('\t'));
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yml).expect("valid yaml");
        assert!(parsed.get("jobs").is_some());
    }

    #[test]
    fn amd64_workflow_builds_natively_without_emulation() {
        let yml = render_deploy_workflow("o", "r", "a", "https://example.com/hooks/deploy", "linux/amd64");
        assert!(yml.contains("platforms: linux/amd64"));
        assert!(!yml.contains("setup-qemu-action"));
    }

    #[test]
    fn arm64_workflow_sets_up_emulation_and_is_valid_yaml() {
        let yml = render_deploy_workflow("o", "r", "a", "https://example.com/hooks/deploy", "linux/arm64");
        assert!(yml.contains("platforms: linux/arm64"));
        assert!(yml.contains("docker/setup-qemu-action@v3"));
        assert!(yml.contains("docker/setup-buildx-action@v3"));
        assert!(!yml.contains("__"));
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yml).expect("valid yaml");
        let steps = parsed["jobs"]["deploy"]["steps"].as_sequence().expect("steps");
        let qemu = steps.iter().position(|s| s["uses"].as_str() == Some("docker/setup-qemu-action@v3"));
        let build = steps.iter().position(|s| s["uses"].as_str() == Some("docker/build-push-action@v6"));
        assert!(qemu.unwrap() < build.unwrap());
    }
}
