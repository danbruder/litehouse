//! Renders the GitHub Actions workflow that litehouse commits into a user's
//! repo on `lh create`. The workflow builds and pushes a container image to
//! GHCR, then calls the litehouse deploy hook with the freshly built image.
//!
//! Placeholders (`__OWNER__`, `__REPO__`, `__APP__`, `__HOOK_URL__`) are substituted via
//! plain string replacement rather than `format!`, since the template is
//! full of literal `${{ ... }}` GitHub Actions expression syntax that would
//! otherwise have to be escaped as `{{{{ ... }}}}`.

const TEMPLATE: &str = r#"name: litehouse deploy
on:
  push:
    branches: [main, master]
jobs:
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
      - uses: docker/build-push-action@v6
        with:
          context: .
          push: true
          tags: |
            ghcr.io/__OWNER__/__REPO__:latest
            ghcr.io/__OWNER__/__REPO__:${{ github.sha }}
      - name: notify litehouse
        run: |
          curl -fsS -X POST "__HOOK_URL__" \
            -H "Authorization: Bearer ${{ secrets.LITEHOUSE_DEPLOY_TOKEN }}" \
            -H "Content-Type: application/json" \
            -d "{\"app\":\"__APP__\",\"image\":\"ghcr.io/__OWNER__/__REPO__:${{ github.sha }}\",\"sha\":\"${{ github.sha }}\"}"
"#;

/// Render the litehouse deploy workflow for `owner/app`, pointed at
/// `hook_url`. GHCR image paths must be lowercase (GitHub requires it), so
/// `owner` and `app` are lowercased wherever they appear in an image
/// reference or the hook payload's `app` field.
/// `owner`/`repo` name the GHCR image (GHCR paths follow the repo and must be
/// lowercase); `app` is the litehouse app name used in the deploy-hook payload
/// — they are NOT the same thing (repo "litehouse-hello" may back app "hello").
pub fn render_deploy_workflow(owner: &str, repo: &str, app: &str, hook_url: &str) -> String {
    let owner = owner.to_lowercase();
    let repo = repo.to_lowercase();

    TEMPLATE
        .replace("__OWNER__", &owner)
        .replace("__REPO__", &repo)
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
        );
        assert!(yml.contains("ghcr.io/danbruder/hello:latest"));
        assert!(yml.contains("ghcr.io/danbruder/hello:${{ github.sha }}"));
        assert!(!yml.contains("ghcr.io/DanBruder"));
        assert!(!yml.contains("ghcr.io/danbruder/Hello"));
    }

    #[test]
    fn workflow_contains_no_tabs_and_is_valid_yaml() {
        let yml =
            render_deploy_workflow("owner", "repo", "app", "https://example.com/hooks/deploy");
        assert!(!yml.contains('\t'));
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yml).expect("valid yaml");
        assert!(parsed.get("jobs").is_some());
    }
}
