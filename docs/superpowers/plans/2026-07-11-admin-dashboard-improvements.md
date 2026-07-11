# Admin Dashboard Improvements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the admin dashboard self-sufficient for the push-to-deploy debugging loop: surface deploy errors and in-flight deploys, flesh out the app detail page, add restart/redeploy/backup actions with visible failure feedback, and fix small UX gaps (logs scroll, logout, empty state).

**Architecture:** All work stays inside the existing server-rendered UI layer (`src/ui.rs` + `templates/` + `src/ui/styles.css`). No schema changes — every feature renders data already in SQLite (`deploy.error`, `deploy.status`, `app.custom_domains`, backup report JSON) or calls existing command/deploy/backup code paths. State-changing actions remain plain HTML form POSTs guarded by the existing CSRF middleware; HTMX is used only for polling (log tail, in-flight deploy refresh). Action failures surface via a `?flash=<code>` query param with a fixed server-side code→message map (no encoding or injection concerns, no session machinery).

**Tech Stack:** Rust, axum 0.6, Askama templates, HTMX (vendored), SQLx/SQLite, chrono. Tests use `tower::ServiceExt::oneshot` like the existing `src/ui.rs` test module (they assume Docker is running, same as the rest of the suite).

**Testing note:** All UI tests live in `src/ui.rs` `#[cfg(test)] mod tests` and reuse the existing helpers `test_state()`, `router()`, `body_string()`, and `TEST_TOKEN`. Run a single test with `cargo test <name> -- --nocapture`. State-changing POSTs must send `Host` + `Origin` headers that match (the CSRF guard) plus the auth cookie — copy the pattern from the existing `state_changing_post_with_matching_origin_passes_guard` test.

---

## File Structure

- Modify: `src/ui.rs` — new helpers (`relative_time`, `flash_message`), new template fields, new routes (`restart`, `redeploy`, `backup/run`, `logout`), form handling with `next`/flash
- Modify: `templates/apps.html` — flash banner, deploy status badge, in-progress polling, richer backups card, empty state
- Modify: `templates/app_detail.html` — header card (state/URL/image/repo/domains), action buttons, deploy error column, log scroll script
- Modify: `templates/base.html` — logout form in header
- Modify: `src/ui/styles.css` — deploy badge colors, flash banner style
- No new files, no migrations.

---

### Task 1: `relative_time` helper

Render deploy timestamps as "3m ago" instead of raw RFC 3339 strings. Pure function, no HTTP.

**Files:**
- Modify: `src/ui.rs` (add helper near `app_url`, tests in existing `mod tests`)

- [ ] **Step 1: Write the failing tests**

Add to `src/ui.rs` `mod tests`:

```rust
#[test]
fn relative_time_formats_ago_buckets() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-07-11T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert_eq!(relative_time_at("2026-07-11T11:59:40Z", now), "just now");
    assert_eq!(relative_time_at("2026-07-11T11:55:00Z", now), "5m ago");
    assert_eq!(relative_time_at("2026-07-11T09:00:00Z", now), "3h ago");
    assert_eq!(relative_time_at("2026-07-08T12:00:00Z", now), "3d ago");
}

#[test]
fn relative_time_passes_through_unparseable_input() {
    let now = chrono::Utc::now();
    assert_eq!(relative_time_at("not-a-date", now), "not-a-date");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test relative_time`
Expected: FAIL to compile — `relative_time_at` not found.

- [ ] **Step 3: Write the implementation**

Add to `src/ui.rs` below `app_url`:

```rust
/// "3m ago"-style rendering of an RFC 3339 timestamp, relative to `now`.
/// Unparseable input is returned verbatim rather than erased — a raw
/// timestamp is still more useful on the dashboard than a blank cell.
fn relative_time_at(rfc3339: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let Ok(ts) = chrono::DateTime::parse_from_rfc3339(rfc3339) else {
        return rfc3339.to_string();
    };
    let secs = (now - ts.with_timezone(&chrono::Utc)).num_seconds().max(0);
    match secs {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

fn relative_time(rfc3339: &str) -> String {
    relative_time_at(rfc3339, chrono::Utc::now())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test relative_time`
Expected: 2 passed. (`relative_time` itself is exercised by later tasks; a `#[allow(dead_code)]` is NOT needed because Task 3 uses it — if you compile between tasks, add `let _ = relative_time;` nowhere; just tolerate the unused warning until Task 3.)

- [ ] **Step 5: Commit**

```bash
git add src/ui.rs
git commit -m "feat(ui): add relative_time helper for dashboard timestamps"
```

---

### Task 2: Flash messages + redirect-back for start/stop

Today `start_app_ui`/`stop_app_ui` swallow errors into `tracing::error!` and always redirect to `/` — a failed start looks identical to a successful one. Add: (a) a hidden `next` form field so actions return to the page they came from, (b) a `?flash=<code>` mechanism with a fixed code→message map rendered as a banner.

**Files:**
- Modify: `src/ui.rs` (form struct, flash map, handlers, template fields, index handler)
- Modify: `templates/apps.html` (flash banner)
- Modify: `src/ui/styles.css` (banner style)

- [ ] **Step 1: Write the failing tests**

Add to `src/ui.rs` `mod tests`:

```rust
#[tokio::test]
async fn stop_failure_redirects_back_to_next_with_flash() {
    let state = test_state().await;
    let app = router(state);

    // "whatever" doesn't exist, so stop fails -> flash code appended.
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/apps/whatever/stop")
                .header(header::HOST, "admin.lh.example.com")
                .header(header::ORIGIN, "https://admin.lh.example.com")
                .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(axum::body::Body::from("next=/apps/whatever"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/apps/whatever?flash=stop-failed"
    );
}

#[tokio::test]
async fn index_renders_flash_message_for_known_code() {
    let state = test_state().await;
    let app = router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/?flash=stop-failed")
                .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
    assert!(body.contains("Failed to stop the app"));
}

#[tokio::test]
async fn unknown_flash_code_renders_nothing() {
    let state = test_state().await;
    let app = router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/?flash=<script>alert(1)</script>")
                .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
    assert!(!body.contains("alert(1)"));
    assert!(!body.contains("class=\"flash\""));
}

#[tokio::test]
async fn next_field_rejects_offsite_redirects() {
    let state = test_state().await;
    let app = router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/apps/whatever/stop")
                .header(header::HOST, "admin.lh.example.com")
                .header(header::ORIGIN, "https://admin.lh.example.com")
                .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(axum::body::Body::from("next=//evil.example.com/phish"))
                .unwrap(),
        )
        .await
        .unwrap();

    // Offsite `next` falls back to "/".
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/?flash=stop-failed"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test flash -- --nocapture && cargo test next_field`
Expected: FAIL (the redirect Location is `/`, no flash rendering exists).

- [ ] **Step 3: Implement flash + next plumbing in `src/ui.rs`**

Add near the top (below `app_url`):

```rust
/// Fixed flash-code -> message map. Codes travel in the `?flash=` query
/// param; anything not in this map renders nothing, so the param is not an
/// injection surface and needs no encoding.
fn flash_message(code: &str) -> Option<&'static str> {
    match code {
        "start-failed" => Some("Failed to start the app — check the server logs."),
        "stop-failed" => Some("Failed to stop the app — check the server logs."),
        "restart-failed" => Some("Failed to restart the app — check the server logs."),
        "redeploy-failed" => Some("Redeploy could not be started — check the server logs."),
        "no-image" => Some("This app has no deployed image yet — push to its repo first."),
        "backup-started" => Some("Backup started — refresh in a minute for the report."),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
struct ActionForm {
    next: Option<String>,
}

/// Only same-site absolute paths are honored; anything else ("//evil…",
/// "https://…", relative paths) falls back to "/".
fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(p) if p.starts_with('/') && !p.starts_with("//") => p.to_string(),
        _ => "/".to_string(),
    }
}

fn redirect_after_action(next: Option<&str>, error_code: Option<&str>) -> Redirect {
    let base = safe_next(next);
    match error_code {
        Some(code) => Redirect::to(&format!("{base}?flash={code}")),
        None => Redirect::to(&base),
    }
}

#[derive(Debug, Deserialize)]
struct FlashQuery {
    flash: Option<String>,
}
```

Replace the `start_app_ui` and `stop_app_ui` handlers (note: `Option<Form<...>>` keeps the existing tests that POST an empty body passing — a missing/unparseable body just means `next = None`):

```rust
async fn start_app_ui(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    form: Option<Form<ActionForm>>,
) -> impl IntoResponse {
    let next = form.as_ref().and_then(|f| f.next.as_deref()).map(str::to_string);
    let (pool, docker) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone())
    };
    let error = match crate::commands::start::execute(&pool, &docker, &name).await {
        Ok(()) => None,
        Err(e) => {
            tracing::error!("ui: failed to start app '{}': {}", name, e);
            Some("start-failed")
        }
    };
    redirect_after_action(next.as_deref(), error)
}

async fn stop_app_ui(
    Path(name): Path<String>,
    form: Option<Form<ActionForm>>,
) -> impl IntoResponse {
    let next = form.as_ref().and_then(|f| f.next.as_deref()).map(str::to_string);
    let error = match crate::commands::stop::execute(&name).await {
        Ok(()) => None,
        Err(e) => {
            tracing::error!("ui: failed to stop app '{}': {}", name, e);
            Some("stop-failed")
        }
    };
    redirect_after_action(next.as_deref(), error)
}
```

Add the import for `Query` — change the `axum::extract` import line to:

```rust
    extract::{Form, Path, Query, State},
```

Add a `flash` field to `AppsTemplate`:

```rust
#[derive(Template)]
#[template(path = "apps.html")]
struct AppsTemplate {
    apps: Vec<AppRow>,
    backups_summary: String,
    flash: Option<String>,
}
```

Change `apps_index`'s signature and final render:

```rust
async fn apps_index(
    State(state): State<Arc<RwLock<AppState>>>,
    Query(q): Query<FlashQuery>,
) -> Response {
```

and

```rust
    HtmlTemplate(AppsTemplate {
        apps: rows,
        backups_summary,
        flash: q
            .flash
            .as_deref()
            .and_then(flash_message)
            .map(str::to_string),
    })
    .into_response()
```

- [ ] **Step 4: Render the banner in `templates/apps.html`**

At the top of the `content` block (before the backups card):

```html
{% if let Some(msg) = flash %}
<div class="flash">{{ msg }}</div>
{% endif %}
```

And update the action forms in the table to carry `next` (stays `/` for the index):

```html
      <td>
        <form class="inline" method="post" action="/apps/{{ app.name }}/start">
          <input type="hidden" name="next" value="/">
          <button type="submit">Start</button>
        </form>
        <form class="inline" method="post" action="/apps/{{ app.name }}/stop">
          <input type="hidden" name="next" value="/">
          <button type="submit">Stop</button>
        </form>
      </td>
```

- [ ] **Step 5: Add the banner style to `src/ui/styles.css`**

```css
.flash {
  background: var(--card-bg);
  border: 1px solid var(--danger);
  color: var(--danger);
  border-radius: 8px;
  padding: 0.6rem 1rem;
  margin-bottom: 1rem;
  font-size: 0.9rem;
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test --lib ui`
Expected: all `ui::tests` pass, including the pre-existing ones (empty-body POSTs still redirect to `/`).

- [ ] **Step 7: Commit**

```bash
git add src/ui.rs templates/apps.html src/ui/styles.css
git commit -m "feat(ui): flash messages and redirect-back for start/stop actions"
```

---

### Task 3: Deploy errors + relative times on the app detail page

The `deploy` table has an `error` column that the UI never renders. Show it in the deploy history, and switch `created_at` to relative time.

**Files:**
- Modify: `src/ui.rs` (`DeployRow`, `app_detail`)
- Modify: `templates/app_detail.html` (error column, status badge)
- Modify: `src/ui/styles.css` (deploy badge colors)

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn app_detail_shows_deploy_error() {
    let state = test_state().await;
    {
        let s = state.read().await;
        let app = App::new("erroring-app").unwrap();
        db::app::save(&s.db_pool, &app).await.unwrap();
        let deploy = crate::models::Deploy::new(&app.id, "ghcr.io/x/erroring-app:sha", Some("abc1234def"));
        db::deploy::insert(&s.db_pool, &deploy).await.unwrap();
        db::deploy::set_status(&s.db_pool, &deploy.id, "failed", Some("failed to pull image: manifest unknown"))
            .await
            .unwrap();
    }
    let app = router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/apps/erroring-app")
                .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
    assert!(body.contains("manifest unknown"));
    assert!(body.contains("badge-deploy-failed"));
    assert!(body.contains("ago") || body.contains("just now"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test app_detail_shows_deploy_error`
Expected: FAIL — error text not in body.

- [ ] **Step 3: Extend `DeployRow` and the `app_detail` handler in `src/ui.rs`**

```rust
struct DeployRow {
    status: String,
    image: String,
    sha: String,
    created_at: String,
    error: Option<String>,
}
```

In `app_detail`, replace the deploys mapping:

```rust
    let deploys = db::deploy::list_for_app(&pool, &app.id, 20)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|d| DeployRow {
            status: d.status,
            image: d.image,
            sha: d
                .git_sha
                .map(|s| s.chars().take(7).collect::<String>())
                .unwrap_or_else(|| "-".to_string()),
            created_at: relative_time(&d.created_at),
            error: d.error,
        })
        .collect();
```

- [ ] **Step 4: Render status badge + error in `templates/app_detail.html`**

Replace the deploys table with:

```html
<table>
  <thead>
    <tr>
      <th>Status</th>
      <th>Image</th>
      <th>SHA</th>
      <th>When</th>
      <th>Error</th>
    </tr>
  </thead>
  <tbody>
    {% for d in deploys %}
    <tr>
      <td><span class="badge badge-deploy-{{ d.status }}">{{ d.status }}</span></td>
      <td>{{ d.image }}</td>
      <td>{{ d.sha }}</td>
      <td>{{ d.created_at }}</td>
      <td class="deploy-error">
        {% if let Some(err) = d.error %}{{ err }}{% else %}<span class="muted">—</span>{% endif %}
      </td>
    </tr>
    {% endfor %}
    {% if deploys.is_empty() %}
    <tr><td colspan="5" class="muted">no deploys yet</td></tr>
    {% endif %}
  </tbody>
</table>
```

- [ ] **Step 5: Add badge + error-cell styles to `src/ui/styles.css`**

```css
.badge-deploy-succeeded { color: var(--ok); }
.badge-deploy-failed { color: var(--danger); }
.badge-deploy-in_progress { color: var(--accent); }

td.deploy-error {
  max-width: 320px;
  overflow-wrap: anywhere;
  color: var(--danger);
  font-size: 0.8rem;
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test --lib ui`
Expected: PASS (including `app_detail_never_renders_env_values` — unrelated but confirms no regression).

- [ ] **Step 7: Commit**

```bash
git add src/ui.rs templates/app_detail.html src/ui/styles.css
git commit -m "feat(ui): surface deploy errors and relative times on app detail"
```

---

### Task 4: Index — deploy status badge, in-progress polling, empty state

Make a failed or in-flight last deploy visually loud on the index, auto-refresh the table while a deploy is in flight (HTMX poll with `hx-select`), and add a first-install empty state.

**Files:**
- Modify: `src/ui.rs` (`AppRow`, `AppsTemplate`, `apps_index`)
- Modify: `templates/apps.html`

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn index_shows_deploy_status_badge_and_polls_while_in_progress() {
    let state = test_state().await;
    {
        let s = state.read().await;
        let app = App::new("deploying-app").unwrap();
        db::app::save(&s.db_pool, &app).await.unwrap();
        // Deploy::new inserts with status "in_progress".
        let deploy = crate::models::Deploy::new(&app.id, "ghcr.io/x/deploying-app:sha", Some("abc1234def"));
        db::deploy::insert(&s.db_pool, &deploy).await.unwrap();
    }
    let app = router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/")
                .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
    assert!(body.contains("badge-deploy-in_progress"));
    assert!(body.contains("hx-trigger=\"every 5s\""));
}

#[tokio::test]
async fn index_without_apps_shows_create_hint() {
    let state = test_state().await;
    let app = router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/")
                .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
    assert!(body.contains("lh create"));
    // No polling attribute when nothing is deploying.
    assert!(!body.contains("hx-trigger=\"every 5s\""));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test index_shows_deploy_status && cargo test index_without_apps`
Expected: FAIL.

- [ ] **Step 3: Extend `AppRow`/`AppsTemplate` and `apps_index` in `src/ui.rs`**

```rust
struct AppRow {
    name: String,
    state: String,
    state_class: String,
    url: String,
    deploy_status: Option<String>,
    last_deploy: String,
}

#[derive(Template)]
#[template(path = "apps.html")]
struct AppsTemplate {
    apps: Vec<AppRow>,
    backups_summary: String,
    flash: Option<String>,
    any_in_progress: bool,
}
```

In `apps_index`, replace the per-app loop body's `last_deploy` block and row push with:

```rust
    let mut any_in_progress = false;
    let mut rows = Vec::with_capacity(apps.len());
    for app in apps {
        let (deploy_status, last_deploy) = match db::deploy::latest_for_app(&pool, &app.id).await {
            Ok(Some(d)) => {
                if d.status == "in_progress" {
                    any_in_progress = true;
                }
                let short_sha = d
                    .git_sha
                    .as_deref()
                    .map(|s| s.chars().take(7).collect::<String>())
                    .unwrap_or_else(|| "-".to_string());
                (
                    Some(d.status),
                    format!("{} · {}", short_sha, relative_time(&d.created_at)),
                )
            }
            Ok(None) => (None, "no deploys".to_string()),
            Err(_) => (None, "unknown".to_string()),
        };

        // Read-through: reflect the live Docker container state rather than
        // the (possibly stale) cached DB column. Fall back to the cached
        // desired state if Docker can't be reached.
        let live = crate::docker::live_state(&app.name)
            .await
            .unwrap_or(app.state);
        let state_str = live.to_string();
        let state_class = state_str.clone();
        rows.push(AppRow {
            name: app.name.clone(),
            state: state_str,
            state_class,
            url: app_url(&app.name, &config),
            deploy_status,
            last_deploy,
        });
    }
```

And pass `any_in_progress` through the final `AppsTemplate { ... }` literal.

- [ ] **Step 4: Update `templates/apps.html`**

Replace the table with:

```html
{% if apps.is_empty() %}
<div class="card">
  <p>No apps yet.</p>
  <p class="muted">Run <code>lh create &lt;app&gt; --repo owner/name</code>, then <code>git push</code> to deploy.</p>
</div>
{% else %}
<table {% if any_in_progress %}hx-get="/" hx-select="table" hx-swap="outerHTML" hx-trigger="every 5s"{% endif %}>
  <thead>
    <tr>
      <th>Name</th>
      <th>State</th>
      <th>URL</th>
      <th>Last Deploy</th>
      <th>Actions</th>
    </tr>
  </thead>
  <tbody>
    {% for app in apps %}
    <tr>
      <td><a href="/apps/{{ app.name }}">{{ app.name }}</a></td>
      <td><span class="badge badge-{{ app.state_class }}">{{ app.state }}</span></td>
      <td><a href="{{ app.url }}" target="_blank" rel="noopener">{{ app.url }}</a></td>
      <td>
        {% if let Some(status) = app.deploy_status %}
        <span class="badge badge-deploy-{{ status }}">{{ status }}</span>
        {% endif %}
        {{ app.last_deploy }}
      </td>
      <td>
        <form class="inline" method="post" action="/apps/{{ app.name }}/start">
          <input type="hidden" name="next" value="/">
          <button type="submit">Start</button>
        </form>
        <form class="inline" method="post" action="/apps/{{ app.name }}/stop">
          <input type="hidden" name="next" value="/">
          <button type="submit">Stop</button>
        </form>
      </td>
    </tr>
    {% endfor %}
  </tbody>
</table>
{% endif %}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib ui`
Expected: PASS. Note `get_root_with_valid_cookie_renders_apps_table` still passes (it saves an app, so the table branch renders).

- [ ] **Step 6: Commit**

```bash
git add src/ui.rs templates/apps.html
git commit -m "feat(ui): deploy status badges, in-flight polling, and empty state on index"
```

---

### Task 5: App detail header card + actions

The detail page currently shows only env names, deploys, and logs. Add a header card with live state, URL, image, repo link, port, and custom domains — plus Start/Stop buttons that redirect back to the detail page.

**Files:**
- Modify: `src/ui.rs` (`AppDetailTemplate`, `app_detail`)
- Modify: `templates/app_detail.html`

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn app_detail_header_card_shows_state_url_and_domains() {
    let state = test_state().await;
    {
        let s = state.read().await;
        let mut app = App::new("card-app").unwrap();
        app.repo = Some("danbruder/card-app".to_string());
        app.image = Some("ghcr.io/danbruder/card-app:abc".to_string());
        app.custom_domains = Some(r#"["cardapp.example.com"]"#.to_string());
        db::app::save(&s.db_pool, &app).await.unwrap();
    }
    let app = router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/apps/card-app")
                .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
    assert!(body.contains("badge-"));                       // state badge
    assert!(body.contains("http://card-app.localhost"));    // app URL (local dev)
    assert!(body.contains("github.com/danbruder/card-app")); // repo link
    assert!(body.contains("ghcr.io/danbruder/card-app:abc")); // image
    assert!(body.contains("cardapp.example.com"));           // custom domain
    assert!(body.contains("/apps/card-app/start"));          // action form
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test app_detail_header_card`
Expected: FAIL.

- [ ] **Step 3: Extend the template struct and handler in `src/ui.rs`**

```rust
#[derive(Template)]
#[template(path = "app_detail.html")]
struct AppDetailTemplate {
    app_name: String,
    state: String,
    state_class: String,
    url: String,
    image: Option<String>,
    repo: Option<String>,
    port: Option<i64>,
    custom_domains: Vec<String>,
    env_names: Vec<String>,
    deploys: Vec<DeployRow>,
    flash: Option<String>,
}
```

In `app_detail`, fetch the server config alongside the pool, add a `Query<FlashQuery>` extractor, compute live state, and fill the new fields:

```rust
async fn app_detail(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    Query(q): Query<FlashQuery>,
) -> Response {
    let (pool, config) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.server_config.clone())
    };
```

then after loading `app`:

```rust
    let live = crate::docker::live_state(&app.name)
        .await
        .unwrap_or(app.state);
    let state_str = live.to_string();
```

and the final render:

```rust
    HtmlTemplate(AppDetailTemplate {
        url: app_url(&app.name, &config),
        state_class: state_str.clone(),
        state: state_str,
        image: app.image.clone(),
        repo: app.repo.clone(),
        port: app.port,
        custom_domains: app.custom_domains_list(),
        app_name: app.name,
        env_names,
        deploys,
        flash: q
            .flash
            .as_deref()
            .and_then(flash_message)
            .map(str::to_string),
    })
    .into_response()
```

(Note field ordering: `app.name` is moved *after* the borrows of `app.image`/`app.repo` clones — struct literal order above compiles because clones happen before the move.)

- [ ] **Step 4: Rewrite the top of `templates/app_detail.html`**

Replace everything from `<p><a href="/">…` down to (but not including) `<h3>Deploys</h3>` with:

```html
<p><a href="/">&larr; all apps</a></p>

{% if let Some(msg) = flash %}
<div class="flash">{{ msg }}</div>
{% endif %}

<h2>{{ app_name }} <span class="badge badge-{{ state_class }}">{{ state }}</span></h2>

<div class="card">
  <p><strong>URL:</strong> <a href="{{ url }}" target="_blank" rel="noopener">{{ url }}</a></p>
  {% if !custom_domains.is_empty() %}
  <p><strong>Domains:</strong>
    {% for d in custom_domains %}<a href="https://{{ d }}" target="_blank" rel="noopener">{{ d }}</a> {% endfor %}
  </p>
  {% endif %}
  {% if let Some(r) = repo %}
  <p><strong>Repo:</strong> <a href="https://github.com/{{ r }}" target="_blank" rel="noopener">{{ r }}</a></p>
  {% endif %}
  {% if let Some(img) = image %}
  <p><strong>Image:</strong> <code>{{ img }}</code></p>
  {% endif %}
  {% if let Some(p) = port %}
  <p><strong>Port:</strong> {{ p }}</p>
  {% endif %}
  <p>
    <form class="inline" method="post" action="/apps/{{ app_name }}/start">
      <input type="hidden" name="next" value="/apps/{{ app_name }}">
      <button type="submit">Start</button>
    </form>
    <form class="inline" method="post" action="/apps/{{ app_name }}/stop">
      <input type="hidden" name="next" value="/apps/{{ app_name }}">
      <button type="submit">Stop</button>
    </form>
  </p>
</div>

<h3>Environment</h3>
<p class="muted">
  {% if env_names.is_empty() %}
  no environment variables set
  {% else %}
  {% for name in env_names %}<span class="badge">{{ name }}</span> {% endfor %}
  {% endif %}
</p>
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib ui`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/ui.rs templates/app_detail.html
git commit -m "feat(ui): app detail header card with state, URL, repo, image, domains, and actions"
```

---

### Task 6: Restart action

`lh restart` exists in the CLI (client-side stop+start); the UI has no equivalent. Add `POST /apps/:name/restart` that stops then starts, with flash on failure.

**Files:**
- Modify: `src/ui.rs` (route + handler)
- Modify: `templates/apps.html`, `templates/app_detail.html` (buttons)

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn restart_unknown_app_redirects_with_flash() {
    let state = test_state().await;
    let app = router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/apps/whatever/restart")
                .header(header::HOST, "admin.lh.example.com")
                .header(header::ORIGIN, "https://admin.lh.example.com")
                .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(axum::body::Body::from("next=/apps/whatever"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/apps/whatever?flash=restart-failed"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test restart_unknown_app`
Expected: FAIL — 404/405, route doesn't exist.

- [ ] **Step 3: Add handler and route in `src/ui.rs`**

Handler (mirrors the CLI's stop-then-start semantics):

```rust
async fn restart_app_ui(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    form: Option<Form<ActionForm>>,
) -> impl IntoResponse {
    let next = form.as_ref().and_then(|f| f.next.as_deref()).map(str::to_string);
    let (pool, docker) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone())
    };
    let result = async {
        crate::commands::stop::execute(&name).await?;
        crate::commands::start::execute(&pool, &docker, &name).await
    }
    .await;
    let error = match result {
        Ok(()) => None,
        Err(e) => {
            tracing::error!("ui: failed to restart app '{}': {}", name, e);
            Some("restart-failed")
        }
    };
    redirect_after_action(next.as_deref(), error)
}
```

Route (in `create_ui_router`, next to start/stop):

```rust
        .route("/apps/:name/restart", post(restart_app_ui))
```

- [ ] **Step 4: Add Restart buttons**

In `templates/apps.html`, after the Stop form:

```html
        <form class="inline" method="post" action="/apps/{{ app.name }}/restart">
          <input type="hidden" name="next" value="/">
          <button type="submit">Restart</button>
        </form>
```

In `templates/app_detail.html`, after the Stop form in the header card:

```html
    <form class="inline" method="post" action="/apps/{{ app_name }}/restart">
      <input type="hidden" name="next" value="/apps/{{ app_name }}">
      <button type="submit">Restart</button>
    </form>
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib ui`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/ui.rs templates/apps.html templates/app_detail.html
git commit -m "feat(ui): restart action on index and app detail"
```

---

### Task 7: Redeploy action

Re-pull the app's current image and recreate the container — the recovery move for a wedged container. Reuses `deploy::deploy_app` (the same path as the deploy hook), so the outcome lands in deploy history where Task 3 already renders errors.

**Files:**
- Modify: `src/ui.rs` (route + handler)
- Modify: `templates/app_detail.html` (button)

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn redeploy_app_without_image_redirects_with_no_image_flash() {
    let state = test_state().await;
    {
        let s = state.read().await;
        let app = App::new("noimage-app").unwrap();
        db::app::save(&s.db_pool, &app).await.unwrap();
    }
    let app = router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/apps/noimage-app/redeploy")
                .header(header::HOST, "admin.lh.example.com")
                .header(header::ORIGIN, "https://admin.lh.example.com")
                .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(axum::body::Body::from("next=/apps/noimage-app"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/apps/noimage-app?flash=no-image"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test redeploy_app_without_image`
Expected: FAIL — route doesn't exist.

- [ ] **Step 3: Add handler and route in `src/ui.rs`**

```rust
async fn redeploy_app_ui(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    form: Option<Form<ActionForm>>,
) -> impl IntoResponse {
    let next = form.as_ref().and_then(|f| f.next.as_deref()).map(str::to_string);
    let (pool, docker) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone())
    };

    let image = match db::app::get_by_name(&pool, &name).await {
        Ok(Some(app)) => app.image,
        _ => None,
    };
    let Some(image) = image else {
        return redirect_after_action(next.as_deref(), Some("no-image"));
    };

    // deploy_app records success/failure in the deploy table; the detail
    // page's deploy history is the primary feedback channel, so only a
    // failure to even start the deploy warrants a flash.
    let error = match crate::deploy::deploy_app(&pool, &docker, &name, &image, None).await {
        Ok(_) => None,
        Err(e) => {
            tracing::error!("ui: failed to redeploy app '{}': {}", name, e);
            Some("redeploy-failed")
        }
    };
    redirect_after_action(next.as_deref(), error)
}
```

Route:

```rust
        .route("/apps/:name/redeploy", post(redeploy_app_ui))
```

- [ ] **Step 4: Add the button to `templates/app_detail.html`** (after Restart in the header card)

```html
    <form class="inline" method="post" action="/apps/{{ app_name }}/redeploy">
      <input type="hidden" name="next" value="/apps/{{ app_name }}">
      <button type="submit">Redeploy</button>
    </form>
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib ui`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/ui.rs templates/app_detail.html
git commit -m "feat(ui): redeploy action re-pulls current image via deploy engine"
```

---

### Task 8: Richer backups card + run-now button

"2 succeeded, 1 failed" hides *which* app failed and why. Render the failed list from the stored `BackupReport` (fields: `succeeded: Vec<String>`, `failed: Vec<(String, String)>`, `ran_at: String`) and add a "Run backup now" button that spawns `backup::run_backup` in the background.

**Files:**
- Modify: `src/ui.rs` (template fields, `apps_index`, new route + handler)
- Modify: `templates/apps.html` (card)

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn index_backups_card_lists_failed_apps() {
    let state = test_state().await;
    {
        let s = state.read().await;
        let report = crate::backup::BackupReport {
            succeeded: vec!["good-app".to_string()],
            failed: vec![("bad-app".to_string(), "S3 upload timed out".to_string())],
            ran_at: "2026-07-10T02:00:00Z".to_string(),
        };
        db::system_config::set_last_backup_report(&s.db_pool, &report)
            .await
            .unwrap();
    }
    let app = router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/")
                .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
    assert!(body.contains("bad-app"));
    assert!(body.contains("S3 upload timed out"));
}

#[tokio::test]
async fn run_backup_now_redirects_with_started_flash() {
    let state = test_state().await;
    let app = router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/backup/run")
                .header(header::HOST, "admin.lh.example.com")
                .header(header::ORIGIN, "https://admin.lh.example.com")
                .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/?flash=backup-started"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test backups_card && cargo test run_backup_now`
Expected: FAIL.

- [ ] **Step 3: Replace the summary string with structured fields in `src/ui.rs`**

Change `AppsTemplate`:

```rust
#[derive(Template)]
#[template(path = "apps.html")]
struct AppsTemplate {
    apps: Vec<AppRow>,
    backup_line: String,
    backup_failures: Vec<(String, String)>,
    flash: Option<String>,
    any_in_progress: bool,
}
```

In `apps_index`, replace the `backups_summary` block with:

```rust
    let (backup_line, backup_failures) =
        match db::system_config::get_last_backup_report(&pool).await {
            Ok(Some(report)) => (
                format!(
                    "{} succeeded, {} failed (last run {})",
                    report.succeeded.len(),
                    report.failed.len(),
                    relative_time(&report.ran_at)
                ),
                report.failed,
            ),
            _ => ("no backup has run yet".to_string(), Vec::new()),
        };
```

and update the template literal to pass `backup_line` and `backup_failures`.

Add the run-now handler and route:

```rust
async fn run_backup_ui(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let (pool, docker) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone())
    };
    // Backups VACUUM every app DB and upload to S3 — far too slow to hold a
    // request open for. Fire and forget; run_backup persists its own report,
    // which the index card shows on the next load.
    tokio::spawn(async move {
        if let Err(e) = crate::backup::run_backup(&pool, &docker).await {
            tracing::error!("ui: manual backup run failed: {e:#}");
        }
    });
    Redirect::to("/?flash=backup-started")
}
```

```rust
        .route("/backup/run", post(run_backup_ui))
```

- [ ] **Step 4: Update the card in `templates/apps.html`**

Replace the existing backups card with:

```html
<div class="card">
  <p>
    <strong>Backups:</strong>
    <span class="muted">{{ backup_line }}</span>
    <form class="inline" method="post" action="/backup/run">
      <button type="submit">Run backup now</button>
    </form>
  </p>
  {% if !backup_failures.is_empty() %}
  <ul>
    {% for f in backup_failures %}
    <li><strong>{{ f.0 }}</strong>: <span class="muted">{{ f.1 }}</span></li>
    {% endfor %}
  </ul>
  {% endif %}
</div>
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib ui`
Expected: PASS. (The `run_backup_now` test's spawned backup will fail harmlessly in the background — no S3 config in tests — which is exactly the fire-and-forget contract.)

- [ ] **Step 6: Commit**

```bash
git add src/ui.rs templates/apps.html
git commit -m "feat(ui): per-app backup failure detail and manual run-now button"
```

---

### Task 9: Logs UX — more lines, pinned scroll

The 5s HTMX poll replaces the log `innerHTML` and resets scroll; 100 lines is too few for a crash loop. Bump to 300 lines and pin scroll to the bottom after each swap.

**Files:**
- Modify: `src/ui.rs` (`log_tail`)
- Modify: `templates/app_detail.html` (script)

- [ ] **Step 1: Make the change** (no meaningful unit test — behavior is browser-side; the existing `log_tail_without_cookie_redirects_to_login` test covers routing)

In `src/ui.rs` `log_tail`, change:

```rust
    match crate::commands::logs::execute(&name, 300, false).await {
```

In `templates/app_detail.html`, after the `<pre class="log" ...>` element add:

```html
<script>
  document.body.addEventListener("htmx:afterSwap", function (e) {
    if (e.target.classList && e.target.classList.contains("log")) {
      e.target.scrollTop = e.target.scrollHeight;
    }
  });
</script>
```

- [ ] **Step 2: Run the tests**

Run: `cargo test --lib ui`
Expected: PASS (no regressions).

- [ ] **Step 3: Commit**

```bash
git add src/ui.rs templates/app_detail.html
git commit -m "feat(ui): 300-line log tail with scroll pinned to bottom"
```

---

### Task 10: Logout

There's no way to clear the auth cookie. Add `POST /logout` (POST, not GET — it's state-changing and must pass the CSRF guard) that expires the cookie and redirects to `/login`, with a small form in the header.

**Files:**
- Modify: `src/ui.rs` (route + handler)
- Modify: `templates/base.html` (header form)

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn logout_expires_cookie_and_redirects_to_login() {
    let state = test_state().await;
    let app = router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/logout")
                .header(header::HOST, "admin.lh.example.com")
                .header(header::ORIGIN, "https://admin.lh.example.com")
                .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/login");
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(set_cookie.contains("litehouse_token=;"));
    assert!(set_cookie.contains("Max-Age=0"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test logout_expires_cookie`
Expected: FAIL — route doesn't exist.

- [ ] **Step 3: Add handler and route in `src/ui.rs`**

```rust
async fn logout() -> Response {
    let mut response = Redirect::to("/login").into_response();
    let cookie = format!("{COOKIE_NAME}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0");
    if let Ok(header_value) = axum::http::HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, header_value);
    }
    response
}
```

Route — add to the **protected** router (an unauthenticated caller has nothing to log out of; the redirect-to-login middleware handles them):

```rust
        .route("/logout", post(logout))
```

- [ ] **Step 4: Add the form to `templates/base.html`**

Replace the `<header>` block with:

```html
  <header>
    <a href="/"><h1>litehouse</h1></a>
    <form method="post" action="/logout">
      <button type="submit">Sign out</button>
    </form>
  </header>
```

(The button also renders on `/login`; posting from there just bounces back to `/login` via the auth middleware — harmless, and not worth a template block.)

- [ ] **Step 5: Run the full check**

Run: `cargo test --lib ui && cargo build`
Expected: all UI tests pass, clean build.

- [ ] **Step 6: Commit**

```bash
git add src/ui.rs templates/base.html
git commit -m "feat(ui): logout clears the admin cookie"
```

---

## Final Verification

- [ ] Run the whole suite: `cargo test` — expected: all pass (Docker must be running).
- [ ] Manual smoke test locally: `LITEHOUSE_LOCAL_DEV=1 cargo run -- serve`, open `http://admin.localhost:9090` (or the dev proxy port), sign in, and click through: index badges, empty-state (fresh DB), app detail card, start/stop/restart/redeploy buttons, backup run-now, log scroll, logout.
- [ ] Ship via the standard flow (see memory: tag → CI image → musl binary → `./dev-deploy.sh`) and verify on `https://admin.lh.danbruder.com` against the live droplet.

## Explicitly Out of Scope (YAGNI for this slice)

- Env var editing from the UI (names-only display stays; values must never render — existing test enforces this)
- Server version / disk usage in the header
- Session-derived cookie values (cookie still carries the admin token)
- Log follow/pause controls beyond scroll pinning
