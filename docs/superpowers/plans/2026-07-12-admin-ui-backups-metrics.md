# Admin UI: Backup Catalog + Resource Metrics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a DB-backed backup catalog page and server-rendered SVG resource-usage charts (server + per-app CPU/memory/disk-or-data-size, 24h raw + 30d hourly) to the litehouse admin UI.

**Architecture:** A new 60s background sampler (`src/metrics.rs`) writes host and per-app CPU/mem/disk readings into a new `metric_sample` table, with an hourly rollup into `metric_hourly` and pruning (24h / 30d retention). A new `backup` table catalogs every successful S3 upload, populated by `src/backup.rs` and kept in sync by its existing pruning. The admin UI (`src/ui.rs` + Askama templates) reads these tables and renders hand-written SVG line/band charts (`src/ui/chart.rs`) — no new JS dependency.

**Tech Stack:** Rust, sqlx (SQLite, compile-time-checked queries + `.sqlx` offline cache), bollard (Docker stats/df), Askama templates, HTMX (unchanged), `/proc` + `df` for host metrics.

**Design doc:** `docs/superpowers/specs/2026-07-12-admin-ui-backups-metrics-design.md`

**Note on schema vs. design doc:** the design doc's `metric_hourly` sketch only listed avg/max columns, but the "30-day view shows a min/max band" requirement needs a lower bound too. This plan adds `cpu_min`/`mem_min`/`disk_min` columns that the design doc omitted — this is the corrected, authoritative schema.

---

## Dev workflow note: sqlx offline cache

This project builds with `SQLX_OFFLINE=true` (see `.env`) — `sqlx::query!`/`query_as!` macros are checked at compile time against the cached schema info in `.sqlx/`, not a live database. Every task that adds a **new** `sqlx::query!`/`query_as!` call ends with a step that regenerates this cache:

```bash
rm -f /Users/dan/projects/litehouse/config/dev.db
DATABASE_URL=sqlite:///Users/dan/projects/litehouse/config/dev.db cargo sqlx database create
DATABASE_URL=sqlite:///Users/dan/projects/litehouse/config/dev.db sqlx migrate run --source migrations
DATABASE_URL=sqlite:///Users/dan/projects/litehouse/config/dev.db cargo sqlx prepare -- --all-targets
```

Run this from `/Users/dan/projects/litehouse`. It must be re-run (and the resulting `.sqlx/*.json` files committed) any time the migration or a query changes shape.

---

### Task 1: Migration — `metric_sample`, `metric_hourly`, `backup` tables

**Files:**
- Create: `migrations/20260712_metrics_and_backup_catalog.sql`

- [ ] **Step 1: Write the migration**

```sql
-- Resource-usage history (server + per-app CPU/mem/disk) and a queryable
-- catalog of backup artifacts uploaded to S3 (see src/metrics.rs, src/backup.rs).

CREATE TABLE metric_sample (
    ts    TEXT NOT NULL,   -- RFC3339 UTC, ~1-minute resolution
    scope TEXT NOT NULL,   -- 'server' or an app id
    cpu_pct    REAL,
    mem_bytes  INTEGER,
    disk_bytes INTEGER,    -- server: filesystem used bytes; app: data volume bytes
    PRIMARY KEY (ts, scope)
);

CREATE INDEX idx_metric_sample_scope_ts ON metric_sample (scope, ts);

CREATE TABLE metric_hourly (
    hour  TEXT NOT NULL,   -- RFC3339, truncated to the top of the hour
    scope TEXT NOT NULL,
    cpu_avg REAL,    cpu_min REAL,    cpu_max REAL,
    mem_avg INTEGER, mem_min INTEGER, mem_max INTEGER,
    disk_avg INTEGER, disk_min INTEGER, disk_max INTEGER,
    samples INTEGER NOT NULL,
    PRIMARY KEY (hour, scope)
);

CREATE INDEX idx_metric_hourly_scope_hour ON metric_hourly (scope, hour);

CREATE TABLE backup (
    id         TEXT PRIMARY KEY,
    app_name   TEXT NOT NULL,        -- app name, or 'litehouse-state' for the state DB snapshot
    s3_key     TEXT NOT NULL UNIQUE,
    size_bytes INTEGER NOT NULL,
    status     TEXT NOT NULL,        -- always 'succeeded' today; failed attempts are not catalogued
    created_at TEXT NOT NULL
);

CREATE INDEX idx_backup_created_at ON backup (created_at);
```

- [ ] **Step 2: Regenerate the sqlx offline cache and confirm the migration applies cleanly**

```bash
cd /Users/dan/projects/litehouse
rm -f config/dev.db
DATABASE_URL=sqlite://config/dev.db cargo sqlx database create
DATABASE_URL=sqlite://config/dev.db sqlx migrate run --source migrations
```

Expected: no errors; `sqlx migrate run` reports the new migration applied.

- [ ] **Step 3: Commit**

```bash
git add migrations/20260712_metrics_and_backup_catalog.sql
git commit -m "feat(db): add metric_sample, metric_hourly, and backup catalog tables"
```

---

### Task 2: Models — `MetricSample`, `MetricHourly`, `BackupRecord`

**Files:**
- Create: `src/models/metric.rs`
- Create: `src/models/backup_record.rs`
- Modify: `src/models/mod.rs`

- [ ] **Step 1: Write the metric models**

`src/models/metric.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, sqlx::FromRow, Serialize, Deserialize)]
pub struct MetricSample {
    pub ts: String,
    pub scope: String,
    pub cpu_pct: Option<f64>,
    pub mem_bytes: Option<i64>,
    pub disk_bytes: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, sqlx::FromRow, Serialize, Deserialize)]
pub struct MetricHourly {
    pub hour: String,
    pub scope: String,
    pub cpu_avg: Option<f64>,
    pub cpu_min: Option<f64>,
    pub cpu_max: Option<f64>,
    pub mem_avg: Option<i64>,
    pub mem_min: Option<i64>,
    pub mem_max: Option<i64>,
    pub disk_avg: Option<i64>,
    pub disk_min: Option<i64>,
    pub disk_max: Option<i64>,
    pub samples: i64,
}
```

- [ ] **Step 2: Write the backup catalog model**

`src/models/backup_record.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, sqlx::FromRow, Serialize, Deserialize)]
pub struct BackupRecord {
    pub id: String,
    pub app_name: String,
    pub s3_key: String,
    pub size_bytes: i64,
    pub status: String,
    pub created_at: String,
}

impl BackupRecord {
    pub fn new(app_name: &str, s3_key: &str, size_bytes: i64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            app_name: app_name.to_string(),
            s3_key: s3_key.to_string(),
            size_bytes,
            status: "succeeded".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}
```

- [ ] **Step 3: Register the new modules**

In `src/models/mod.rs`, add (alphabetically alongside the existing `pub mod` lines):

```rust
pub mod backup_record;
pub use backup_record::*;
pub mod metric;
pub use metric::*;
```

- [ ] **Step 4: Build**

```bash
cd /Users/dan/projects/litehouse && cargo build 2>&1 | tail -30
```

Expected: compiles (these types aren't used anywhere yet, so only unused-import/dead-code warnings if any — no errors).

- [ ] **Step 5: Commit**

```bash
git add src/models/metric.rs src/models/backup_record.rs src/models/mod.rs
git commit -m "feat(models): add MetricSample, MetricHourly, BackupRecord"
```

---

### Task 3: `src/db/metrics.rs` — sample storage, rollup, pruning

**Files:**
- Create: `src/db/metrics.rs`
- Modify: `src/db/mod.rs`

- [ ] **Step 1: Write the module**

`src/db/metrics.rs`:

```rust
use super::*;
use crate::models::{MetricHourly, MetricSample};

/// Insert (or, for the same `(ts, scope)`, replace) one sample row.
#[instrument(skip(pool))]
pub async fn insert_sample(
    pool: &Pool<Sqlite>,
    ts: &str,
    scope: &str,
    cpu_pct: Option<f64>,
    mem_bytes: Option<i64>,
    disk_bytes: Option<i64>,
) -> Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO metric_sample (ts, scope, cpu_pct, mem_bytes, disk_bytes)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(ts, scope) DO UPDATE SET
            cpu_pct = excluded.cpu_pct,
            mem_bytes = excluded.mem_bytes,
            disk_bytes = excluded.disk_bytes
        "#,
        ts,
        scope,
        cpu_pct,
        mem_bytes,
        disk_bytes,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Raw samples for `scope` at or after `since` (RFC3339), oldest first.
#[instrument(skip(pool))]
pub async fn list_samples_since(
    pool: &Pool<Sqlite>,
    scope: &str,
    since: &str,
) -> Result<Vec<MetricSample>> {
    let rows = sqlx::query_as!(
        MetricSample,
        r#"
        SELECT ts, scope, cpu_pct, mem_bytes, disk_bytes
        FROM metric_sample
        WHERE scope = ? AND ts >= ?
        ORDER BY ts ASC
        "#,
        scope,
        since,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Hourly rollups for `scope` at or after `since` (RFC3339), oldest first.
#[instrument(skip(pool))]
pub async fn list_hourly_since(
    pool: &Pool<Sqlite>,
    scope: &str,
    since: &str,
) -> Result<Vec<MetricHourly>> {
    let rows = sqlx::query_as!(
        MetricHourly,
        r#"
        SELECT hour, scope, cpu_avg, cpu_min, cpu_max,
               mem_avg, mem_min, mem_max,
               disk_avg, disk_min, disk_max, samples
        FROM metric_hourly
        WHERE scope = ? AND hour >= ?
        ORDER BY hour ASC
        "#,
        scope,
        since,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Every scope with at least one sample in `[hour_start, hour_end)`.
#[instrument(skip(pool))]
async fn scopes_sampled_in_hour(
    pool: &Pool<Sqlite>,
    hour_start: &str,
    hour_end: &str,
) -> Result<Vec<String>> {
    let rows = sqlx::query!(
        r#"SELECT DISTINCT scope FROM metric_sample WHERE ts >= ? AND ts < ?"#,
        hour_start,
        hour_end,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.scope).collect())
}

/// Roll every scope's samples in `[hour_start, hour_end)` up into one
/// `metric_hourly` row per scope (idempotent — safe to re-run for the same
/// hour, e.g. after a server restart mid-hour).
#[instrument(skip(pool))]
pub async fn rollup_hour(pool: &Pool<Sqlite>, hour_start: &str, hour_end: &str) -> Result<()> {
    let scopes = scopes_sampled_in_hour(pool, hour_start, hour_end).await?;
    for scope in scopes {
        sqlx::query!(
            r#"
            INSERT INTO metric_hourly (
                hour, scope,
                cpu_avg, cpu_min, cpu_max,
                mem_avg, mem_min, mem_max,
                disk_avg, disk_min, disk_max,
                samples
            )
            SELECT
                ?, ?,
                AVG(cpu_pct), MIN(cpu_pct), MAX(cpu_pct),
                CAST(AVG(mem_bytes) AS INTEGER), MIN(mem_bytes), MAX(mem_bytes),
                CAST(AVG(disk_bytes) AS INTEGER), MIN(disk_bytes), MAX(disk_bytes),
                COUNT(*)
            FROM metric_sample
            WHERE scope = ? AND ts >= ? AND ts < ?
            ON CONFLICT(hour, scope) DO UPDATE SET
                cpu_avg = excluded.cpu_avg, cpu_min = excluded.cpu_min, cpu_max = excluded.cpu_max,
                mem_avg = excluded.mem_avg, mem_min = excluded.mem_min, mem_max = excluded.mem_max,
                disk_avg = excluded.disk_avg, disk_min = excluded.disk_min, disk_max = excluded.disk_max,
                samples = excluded.samples
            "#,
            hour_start,
            scope,
            scope,
            hour_start,
            hour_end,
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}

#[instrument(skip(pool))]
pub async fn prune_samples_older_than(pool: &Pool<Sqlite>, cutoff: &str) -> Result<()> {
    sqlx::query!(r#"DELETE FROM metric_sample WHERE ts < ?"#, cutoff)
        .execute(pool)
        .await?;
    Ok(())
}

#[instrument(skip(pool))]
pub async fn prune_hourly_older_than(pool: &Pool<Sqlite>, cutoff: &str) -> Result<()> {
    sqlx::query!(r#"DELETE FROM metric_hourly WHERE hour < ?"#, cutoff)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;

    #[tokio::test]
    async fn insert_and_list_samples_since() {
        let pool = get_test_pool().await;
        insert_sample(&pool, "2026-07-12T10:00:00+00:00", "server", Some(12.5), Some(1000), Some(2000))
            .await
            .unwrap();
        insert_sample(&pool, "2026-07-12T10:01:00+00:00", "server", Some(15.0), Some(1100), Some(2000))
            .await
            .unwrap();
        insert_sample(&pool, "2026-07-12T09:00:00+00:00", "server", Some(5.0), Some(900), Some(2000))
            .await
            .unwrap();

        let rows = list_samples_since(&pool, "server", "2026-07-12T10:00:00+00:00")
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].ts, "2026-07-12T10:00:00+00:00");
        assert_eq!(rows[1].ts, "2026-07-12T10:01:00+00:00");
    }

    #[tokio::test]
    async fn insert_sample_same_key_replaces_row() {
        let pool = get_test_pool().await;
        insert_sample(&pool, "2026-07-12T10:00:00+00:00", "server", Some(10.0), None, None)
            .await
            .unwrap();
        insert_sample(&pool, "2026-07-12T10:00:00+00:00", "server", Some(20.0), None, None)
            .await
            .unwrap();

        let rows = list_samples_since(&pool, "server", "2026-07-12T00:00:00+00:00")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cpu_pct, Some(20.0));
    }

    #[tokio::test]
    async fn rollup_hour_computes_avg_min_max_per_scope() {
        let pool = get_test_pool().await;
        insert_sample(&pool, "2026-07-12T10:00:00+00:00", "server", Some(10.0), Some(1000), Some(500))
            .await
            .unwrap();
        insert_sample(&pool, "2026-07-12T10:30:00+00:00", "server", Some(20.0), Some(2000), Some(500))
            .await
            .unwrap();
        // A sample in the next hour must not be included.
        insert_sample(&pool, "2026-07-12T11:00:00+00:00", "server", Some(99.0), Some(9999), Some(500))
            .await
            .unwrap();

        rollup_hour(&pool, "2026-07-12T10:00:00+00:00", "2026-07-12T11:00:00+00:00")
            .await
            .unwrap();

        let rows = list_hourly_since(&pool, "server", "2026-07-12T00:00:00+00:00")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let hour = &rows[0];
        assert_eq!(hour.hour, "2026-07-12T10:00:00+00:00");
        assert_eq!(hour.samples, 2);
        assert_eq!(hour.cpu_avg, Some(15.0));
        assert_eq!(hour.cpu_min, Some(10.0));
        assert_eq!(hour.cpu_max, Some(20.0));
        assert_eq!(hour.mem_avg, Some(1500));
        assert_eq!(hour.disk_avg, Some(500));
    }

    #[tokio::test]
    async fn rollup_hour_is_idempotent() {
        let pool = get_test_pool().await;
        insert_sample(&pool, "2026-07-12T10:00:00+00:00", "server", Some(10.0), Some(1000), Some(500))
            .await
            .unwrap();

        rollup_hour(&pool, "2026-07-12T10:00:00+00:00", "2026-07-12T11:00:00+00:00")
            .await
            .unwrap();
        rollup_hour(&pool, "2026-07-12T10:00:00+00:00", "2026-07-12T11:00:00+00:00")
            .await
            .unwrap();

        let rows = list_hourly_since(&pool, "server", "2026-07-12T00:00:00+00:00")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn prune_samples_and_hourly_removes_only_old_rows() {
        let pool = get_test_pool().await;
        insert_sample(&pool, "2026-07-01T00:00:00+00:00", "server", Some(1.0), None, None)
            .await
            .unwrap();
        insert_sample(&pool, "2026-07-12T00:00:00+00:00", "server", Some(2.0), None, None)
            .await
            .unwrap();
        prune_samples_older_than(&pool, "2026-07-11T00:00:00+00:00")
            .await
            .unwrap();

        let rows = list_samples_since(&pool, "server", "2026-01-01T00:00:00+00:00")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ts, "2026-07-12T00:00:00+00:00");

        rollup_hour(&pool, "2026-07-12T00:00:00+00:00", "2026-07-12T01:00:00+00:00")
            .await
            .unwrap();
        prune_hourly_older_than(&pool, "2026-08-01T00:00:00+00:00")
            .await
            .unwrap();
        let hourly = list_hourly_since(&pool, "server", "2026-01-01T00:00:00+00:00")
            .await
            .unwrap();
        assert!(hourly.is_empty());
    }
}
```

- [ ] **Step 2: Register the module**

In `src/db/mod.rs`, add alongside the other `pub mod` lines:

```rust
pub mod metrics;
```

- [ ] **Step 3: Regenerate the sqlx cache (new queries added)**

```bash
cd /Users/dan/projects/litehouse
DATABASE_URL=sqlite://config/dev.db cargo sqlx prepare -- --all-targets
```

- [ ] **Step 4: Run the tests**

```bash
cargo test db::metrics:: -- --nocapture
```

Expected: all 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/db/metrics.rs src/db/mod.rs .sqlx
git commit -m "feat(db): add metrics sample storage, hourly rollup, and pruning"
```

---

### Task 4: `src/db/backup.rs` — backup catalog queries

**Files:**
- Create: `src/db/backup.rs`
- Modify: `src/db/mod.rs`

- [ ] **Step 1: Write the module**

`src/db/backup.rs`:

```rust
use super::*;
use crate::models::BackupRecord;

/// Record (or, for an already-catalogued S3 key, update) a successfully
/// uploaded backup artifact.
#[instrument(skip(pool))]
pub async fn record_upload(
    pool: &Pool<Sqlite>,
    app_name: &str,
    s3_key: &str,
    size_bytes: i64,
) -> Result<()> {
    let record = BackupRecord::new(app_name, s3_key, size_bytes);
    sqlx::query!(
        r#"
        INSERT INTO backup (id, app_name, s3_key, size_bytes, status, created_at)
        VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(s3_key) DO UPDATE SET
            size_bytes = excluded.size_bytes,
            status = excluded.status,
            created_at = excluded.created_at
        "#,
        record.id,
        record.app_name,
        record.s3_key,
        record.size_bytes,
        record.status,
        record.created_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Every catalogued backup, newest first.
#[instrument(skip(pool))]
pub async fn list_all(pool: &Pool<Sqlite>) -> Result<Vec<BackupRecord>> {
    let rows = sqlx::query_as!(
        BackupRecord,
        r#"SELECT id, app_name, s3_key, size_bytes, status, created_at FROM backup ORDER BY created_at DESC"#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Remove catalog rows for the given S3 keys — called alongside S3-side
/// pruning so the catalog never lists a backup that no longer exists in S3.
#[instrument(skip(pool, keys))]
pub async fn delete_by_keys(pool: &Pool<Sqlite>, keys: &[String]) -> Result<()> {
    for key in keys {
        sqlx::query!(r#"DELETE FROM backup WHERE s3_key = ?"#, key)
            .execute(pool)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;

    #[tokio::test]
    async fn record_and_list_backups_newest_first() {
        let pool = get_test_pool().await;
        record_upload(&pool, "app-a", "apps/app-a/2026-07-10.tar.gz", 1000)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        record_upload(&pool, "app-a", "apps/app-a/2026-07-11.tar.gz", 2000)
            .await
            .unwrap();

        let rows = list_all(&pool).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].s3_key, "apps/app-a/2026-07-11.tar.gz");
    }

    #[tokio::test]
    async fn record_upload_same_key_replaces_row() {
        let pool = get_test_pool().await;
        record_upload(&pool, "app-a", "apps/app-a/2026-07-10.tar.gz", 1000)
            .await
            .unwrap();
        record_upload(&pool, "app-a", "apps/app-a/2026-07-10.tar.gz", 1500)
            .await
            .unwrap();

        let rows = list_all(&pool).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].size_bytes, 1500);
    }

    #[tokio::test]
    async fn delete_by_keys_removes_matching_rows_only() {
        let pool = get_test_pool().await;
        record_upload(&pool, "app-a", "apps/app-a/2026-07-10.tar.gz", 1000)
            .await
            .unwrap();
        record_upload(&pool, "app-b", "apps/app-b/2026-07-10.tar.gz", 2000)
            .await
            .unwrap();

        delete_by_keys(&pool, &["apps/app-a/2026-07-10.tar.gz".to_string()])
            .await
            .unwrap();

        let rows = list_all(&pool).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].app_name, "app-b");
    }
}
```

- [ ] **Step 2: Register the module**

In `src/db/mod.rs`, add:

```rust
pub mod backup;
```

- [ ] **Step 3: Regenerate the sqlx cache**

```bash
cd /Users/dan/projects/litehouse
DATABASE_URL=sqlite://config/dev.db cargo sqlx prepare -- --all-targets
```

- [ ] **Step 4: Run the tests**

```bash
cargo test db::backup:: -- --nocapture
```

Expected: all 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/db/backup.rs src/db/mod.rs .sqlx
git commit -m "feat(db): add backup catalog queries"
```

---

### Task 5: `src/ui/chart.rs` — SVG chart helper

**Files:**
- Create: `src/ui/chart.rs`
- Modify: `src/ui.rs` (add `mod chart;`)

- [ ] **Step 1: Write the module**

`src/ui/chart.rs`:

```rust
//! Minimal server-rendered SVG line/band charts for the admin UI. No
//! client-side JS beyond the HTMX already vendored alongside this module —
//! every chart is a plain inline `<svg>` string built from a series of
//! optional values (a `None` is a tick where sampling failed or hadn't
//! started yet, and breaks the line rather than interpolating across it).

const WIDTH: f64 = 640.0;
const HEIGHT: f64 = 120.0;
const PAD: f64 = 4.0;

#[derive(Debug, Clone, Copy)]
pub enum ChartUnit {
    Percent,
    Bytes,
}

fn format_label(value: f64, unit: ChartUnit) -> String {
    match unit {
        ChartUnit::Percent => format!("{:.1}%", value),
        ChartUnit::Bytes => format_bytes(value as i64),
    }
}

/// Render a single line chart. Returns a placeholder message instead of an
/// empty `<svg>` when the series has no numeric points at all.
pub fn line_chart(points: &[Option<f64>], unit: ChartUnit) -> String {
    let values: Vec<f64> = points.iter().filter_map(|p| *p).collect();
    if values.is_empty() {
        return "<p class=\"muted chart-empty\">no data yet</p>".to_string();
    }

    let max_v = values.iter().cloned().fold(f64::MIN, f64::max).max(1.0);
    let path = build_path(points, 0.0, max_v);
    let current = points.iter().rev().find_map(|p| *p);
    let label = current.map(|v| format_label(v, unit)).unwrap_or_else(|| "—".to_string());

    format!(
        r#"<div class="chart"><svg viewBox="0 0 {w} {h}" preserveAspectRatio="none" class="chart-svg">{path}</svg><span class="chart-label">{label}</span></div>"#,
        w = WIDTH,
        h = HEIGHT,
        path = path,
    )
}

/// Render an average line with a translucent min/max band behind it (used
/// for the 30-day hourly-rollup view).
pub fn band_chart(avg: &[Option<f64>], min: &[Option<f64>], max: &[Option<f64>], unit: ChartUnit) -> String {
    let all_values: Vec<f64> = max.iter().chain(avg.iter()).filter_map(|p| *p).collect();
    if all_values.is_empty() {
        return "<p class=\"muted chart-empty\">no data yet</p>".to_string();
    }
    let max_v = all_values.iter().cloned().fold(f64::MIN, f64::max).max(1.0);

    let band = build_band_path(min, max, 0.0, max_v);
    let avg_path = build_path(avg, 0.0, max_v);
    let current = avg.iter().rev().find_map(|p| *p);
    let label = current
        .map(|v| format!("{} avg", format_label(v, unit)))
        .unwrap_or_else(|| "—".to_string());

    format!(
        r#"<div class="chart"><svg viewBox="0 0 {w} {h}" preserveAspectRatio="none" class="chart-svg">{band}{avg_path}</svg><span class="chart-label">{label}</span></div>"#,
        w = WIDTH,
        h = HEIGHT,
        band = band,
        avg_path = avg_path,
    )
}

fn x_for(i: usize, len: usize) -> f64 {
    if len <= 1 {
        return PAD;
    }
    PAD + (i as f64 / (len - 1) as f64) * (WIDTH - 2.0 * PAD)
}

fn y_for(v: f64, min_v: f64, max_v: f64) -> f64 {
    let span = (max_v - min_v).max(f64::EPSILON);
    let frac = ((v - min_v) / span).clamp(0.0, 1.0);
    HEIGHT - PAD - frac * (HEIGHT - 2.0 * PAD)
}

/// One `<polyline>` per contiguous run of `Some` values — a run of length 1
/// (a lone point surrounded by gaps) is dropped, since a polyline needs at
/// least two points to draw anything.
fn build_path(points: &[Option<f64>], min_v: f64, max_v: f64) -> String {
    let len = points.len();
    let mut segments = String::new();
    let mut current_segment: Vec<String> = Vec::new();

    for (i, p) in points.iter().enumerate() {
        match p {
            Some(v) => {
                let x = x_for(i, len);
                let y = y_for(*v, min_v, max_v);
                current_segment.push(format!("{:.1},{:.1}", x, y));
            }
            None => {
                flush_segment(&mut current_segment, &mut segments);
            }
        }
    }
    flush_segment(&mut current_segment, &mut segments);
    segments
}

fn flush_segment(segment: &mut Vec<String>, out: &mut String) {
    if segment.len() >= 2 {
        out.push_str(&format!(
            r#"<polyline points="{}" fill="none" stroke="currentColor" stroke-width="1.5" />"#,
            segment.join(" ")
        ));
    }
    segment.clear();
}

/// Filled polygon between `min` and `max`, only over indices where both are
/// present.
fn build_band_path(min: &[Option<f64>], max: &[Option<f64>], min_v: f64, max_v: f64) -> String {
    let len = min.len().min(max.len());
    let mut top: Vec<String> = Vec::new();
    let mut bottom: Vec<String> = Vec::new();

    for i in 0..len {
        if let (Some(lo), Some(hi)) = (min[i], max[i]) {
            let x = x_for(i, len);
            top.push(format!("{:.1},{:.1}", x, y_for(hi, min_v, max_v)));
            bottom.push(format!("{:.1},{:.1}", x, y_for(lo, min_v, max_v)));
        }
    }
    if top.len() < 2 {
        return String::new();
    }
    bottom.reverse();
    let mut all_points = top;
    all_points.extend(bottom);
    format!(
        r#"<polygon points="{}" fill="currentColor" opacity="0.15" />"#,
        all_points.join(" ")
    )
}

/// Humanize a byte count (`1.2 GB`, `340 MB`, `512 B`).
pub fn format_bytes(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes <= 0 {
        return "0 B".to_string();
    }
    let mut value = bytes as f64;
    let mut unit_idx = 0;
    while value >= 1024.0 && unit_idx < UNITS.len() - 1 {
        value /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", value, UNITS[unit_idx])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_scales_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(1_500_000_000), "1.4 GB");
    }

    #[test]
    fn line_chart_empty_series_shows_placeholder() {
        let out = line_chart(&[None, None], ChartUnit::Percent);
        assert!(out.contains("no data yet"));
    }

    #[test]
    fn line_chart_renders_polyline_and_current_label() {
        let out = line_chart(&[Some(10.0), Some(20.0), Some(15.0)], ChartUnit::Percent);
        assert!(out.contains("<polyline"));
        assert!(out.contains("15.0%"));
    }

    #[test]
    fn line_chart_drops_isolated_single_point_segments() {
        let out = line_chart(&[Some(10.0), None, Some(20.0)], ChartUnit::Percent);
        // Each side of the gap is only one point — no polyline can be drawn.
        assert!(!out.contains("<polyline"));
    }

    #[test]
    fn band_chart_empty_shows_placeholder() {
        let out = band_chart(&[None], &[None], &[None], ChartUnit::Bytes);
        assert!(out.contains("no data yet"));
    }

    #[test]
    fn band_chart_renders_polygon_and_avg_line() {
        let avg = vec![Some(50.0), Some(60.0)];
        let min = vec![Some(40.0), Some(45.0)];
        let max = vec![Some(60.0), Some(70.0)];
        let out = band_chart(&avg, &min, &max, ChartUnit::Percent);
        assert!(out.contains("<polygon"));
        assert!(out.contains("<polyline"));
        assert!(out.contains("60.0% avg"));
    }
}
```

- [ ] **Step 2: Wire the module into `ui.rs`**

In `src/ui.rs`, add near the top (after the existing `use` block, before `const COOKIE_NAME`):

```rust
mod chart;
```

- [ ] **Step 3: Run the tests**

```bash
cd /Users/dan/projects/litehouse && cargo test ui::chart:: -- --nocapture
```

Expected: all 6 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/ui/chart.rs src/ui.rs
git commit -m "feat(ui): add server-rendered SVG line/band chart helper"
```

---

### Task 6: `src/metrics.rs` — pure calculation helpers (CPU%, mem, disk parsing)

**Files:**
- Create: `src/metrics.rs`
- Modify: `src/lib.rs` (add `pub mod metrics;`)

- [ ] **Step 1: Write the pure functions and their tests**

`src/metrics.rs`:

```rust
//! Resource metrics: pure parsing/calculation helpers (this file) plus the
//! async sampler driver (Task 7) wired into `lh serve` (Task 8).

use bollard::container::CPUStats;

/// Parsed fields from the aggregate `cpu` line of `/proc/stat` (USER_HZ
/// jiffies since boot).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcStatCpu {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
}

impl ProcStatCpu {
    fn total(&self) -> u64 {
        self.user + self.nice + self.system + self.idle + self.iowait + self.irq + self.softirq + self.steal
    }
    fn idle_total(&self) -> u64 {
        self.idle + self.iowait
    }
}

/// Parse the first line of `/proc/stat` (starts with `cpu `). `None` if the
/// line is missing or has fewer fields than expected.
pub fn parse_proc_stat_cpu_line(contents: &str) -> Option<ProcStatCpu> {
    let line = contents.lines().find(|l| l.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map(|f| f.parse::<u64>().ok())
        .collect::<Option<Vec<u64>>>()?;
    if fields.len() < 7 {
        return None;
    }
    Some(ProcStatCpu {
        user: fields[0],
        nice: fields[1],
        system: fields[2],
        idle: fields[3],
        iowait: fields[4],
        irq: fields[5],
        softirq: fields[6],
        steal: fields.get(7).copied().unwrap_or(0),
    })
}

/// CPU percent over the interval between two `/proc/stat` readings
/// ("1 - idle_delta/total_delta"). `None` if no time elapsed or the
/// readings moved backwards (e.g. counter reset).
pub fn cpu_pct_from_proc_stat(prev: &ProcStatCpu, curr: &ProcStatCpu) -> Option<f64> {
    let total_delta = curr.total().checked_sub(prev.total())?;
    let idle_delta = curr.idle_total().checked_sub(prev.idle_total())?;
    if total_delta == 0 {
        return None;
    }
    let busy_delta = total_delta.saturating_sub(idle_delta);
    Some((busy_delta as f64 / total_delta as f64) * 100.0)
}

/// Parse `MemTotal`/`MemAvailable` (in kB) out of `/proc/meminfo` into
/// `(used_bytes, total_bytes)`.
pub fn mem_usage_from_meminfo(contents: &str) -> Option<(i64, i64)> {
    let mut total_kb = None;
    let mut available_kb = None;
    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total_kb = rest.split_whitespace().next().and_then(|s| s.parse::<i64>().ok());
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            available_kb = rest.split_whitespace().next().and_then(|s| s.parse::<i64>().ok());
        }
    }
    let total_kb = total_kb?;
    let available_kb = available_kb?;
    let used_kb = (total_kb - available_kb).max(0);
    Some((used_kb * 1024, total_kb * 1024))
}

/// Parse the second line of `df -B1 <path>` output into `(used_bytes,
/// total_bytes)`. Columns: Filesystem, 1B-blocks, Used, Available, Use%, Mounted.
pub fn parse_df_output(stdout: &str) -> Option<(i64, i64)> {
    let data_line = stdout.lines().nth(1)?;
    let fields: Vec<&str> = data_line.split_whitespace().collect();
    if fields.len() < 3 {
        return None;
    }
    let total = fields[1].parse::<i64>().ok()?;
    let used = fields[2].parse::<i64>().ok()?;
    Some((used, total))
}

/// Docker's own CPU% formula: `cpu_delta / system_delta * online_cpus * 100`.
/// `None` when either delta is non-positive (e.g. a container's very first
/// sample).
pub fn cpu_pct_from_docker_stats(curr: &CPUStats, prev: &CPUStats) -> Option<f64> {
    let cpu_delta = curr.cpu_usage.total_usage.checked_sub(prev.cpu_usage.total_usage)?;
    let system_delta = curr.system_cpu_usage?.checked_sub(prev.system_cpu_usage?)?;
    if cpu_delta == 0 || system_delta == 0 {
        return None;
    }
    let online_cpus = curr.online_cpus.unwrap_or(1).max(1) as f64;
    Some((cpu_delta as f64 / system_delta as f64) * online_cpus * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu(user: u64, idle: u64) -> ProcStatCpu {
        ProcStatCpu { user, nice: 0, system: 0, idle, iowait: 0, irq: 0, softirq: 0, steal: 0 }
    }

    #[test]
    fn parse_proc_stat_cpu_line_reads_first_seven_fields() {
        let contents = "cpu  100 0 50 850 0 0 0 0\ncpu0 100 0 50 850 0 0 0 0\n";
        let parsed = parse_proc_stat_cpu_line(contents).unwrap();
        assert_eq!(parsed.user, 100);
        assert_eq!(parsed.system, 50);
        assert_eq!(parsed.idle, 850);
    }

    #[test]
    fn parse_proc_stat_cpu_line_missing_returns_none() {
        assert!(parse_proc_stat_cpu_line("not stat data").is_none());
    }

    #[test]
    fn cpu_pct_from_proc_stat_computes_busy_fraction() {
        // 100 busy jiffies elapsed out of 1000 total -> 10%.
        let prev = cpu(0, 0);
        let curr = cpu(100, 900);
        assert_eq!(cpu_pct_from_proc_stat(&prev, &curr), Some(10.0));
    }

    #[test]
    fn cpu_pct_from_proc_stat_zero_elapsed_returns_none() {
        let snapshot = cpu(100, 900);
        assert_eq!(cpu_pct_from_proc_stat(&snapshot, &snapshot), None);
    }

    #[test]
    fn mem_usage_from_meminfo_computes_used_as_total_minus_available() {
        let contents = "MemTotal:       16384000 kB\nMemAvailable:    8192000 kB\n";
        let (used, total) = mem_usage_from_meminfo(contents).unwrap();
        assert_eq!(total, 16384000 * 1024);
        assert_eq!(used, 8192000 * 1024);
    }

    #[test]
    fn mem_usage_from_meminfo_missing_field_returns_none() {
        assert!(mem_usage_from_meminfo("MemTotal: 1000 kB\n").is_none());
    }

    #[test]
    fn parse_df_output_reads_used_and_total() {
        let stdout = "Filesystem      1B-blocks       Used   Available Use% Mounted on\n/dev/sda1  25000000000 4100000000 20000000000  18% /\n";
        let (used, total) = parse_df_output(stdout).unwrap();
        assert_eq!(total, 25_000_000_000);
        assert_eq!(used, 4_100_000_000);
    }

    #[test]
    fn parse_df_output_missing_data_line_returns_none() {
        assert!(parse_df_output("Filesystem 1B-blocks Used\n").is_none());
    }

    fn docker_cpu(total_usage: u64, system_cpu_usage: u64, online_cpus: u64) -> CPUStats {
        CPUStats {
            cpu_usage: bollard::container::CPUUsage {
                percpu_usage: None,
                usage_in_usermode: 0,
                total_usage,
                usage_in_kernelmode: 0,
            },
            system_cpu_usage: Some(system_cpu_usage),
            online_cpus: Some(online_cpus),
            throttling_data: bollard::container::ThrottlingData { periods: 0, throttled_periods: 0, throttled_time: 0 },
        }
    }

    #[test]
    fn cpu_pct_from_docker_stats_computes_percentage() {
        let prev = docker_cpu(1_000_000_000, 10_000_000_000, 2);
        let curr = docker_cpu(1_200_000_000, 11_000_000_000, 2);
        // cpu_delta=200_000_000, system_delta=1_000_000_000 -> 0.2 * 2 * 100 = 40%
        assert_eq!(cpu_pct_from_docker_stats(&curr, &prev), Some(40.0));
    }

    #[test]
    fn cpu_pct_from_docker_stats_no_elapsed_time_returns_none() {
        let snapshot = docker_cpu(1_000_000_000, 10_000_000_000, 2);
        assert_eq!(cpu_pct_from_docker_stats(&snapshot, &snapshot), None);
    }
}
```

- [ ] **Step 2: Register the module**

In `src/lib.rs`, add `pub mod metrics;` alongside the other top-level `pub mod` declarations (check the file first to match existing ordering/style).

- [ ] **Step 3: Run the tests**

```bash
cd /Users/dan/projects/litehouse && cargo test metrics:: -- --nocapture
```

Expected: all 10 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/metrics.rs src/lib.rs
git commit -m "feat(metrics): add pure CPU/mem/disk calculation helpers"
```

---

### Task 7: `src/metrics.rs` — async sampler driver, rollup/prune driver

**Files:**
- Modify: `src/metrics.rs` (append to the file from Task 6)

- [ ] **Step 1: Add the async I/O wrappers and sampler**

Append to `src/metrics.rs` (after the pure functions, before the `#[cfg(test)]` block — move the existing test module to the very end of the file if needed):

```rust
use anyhow::{anyhow, Result};
use bollard::container::{Stats, StatsOptions};
use bollard::Docker;
use chrono::Timelike;
use futures_util::StreamExt;
use std::collections::HashMap;
use tracing::warn;

use crate::{config, db};

/// Live host memory usage: `(used_bytes, total_bytes)`.
pub async fn mem_usage() -> Result<(i64, i64)> {
    let contents = tokio::fs::read_to_string("/proc/meminfo").await?;
    mem_usage_from_meminfo(&contents).ok_or_else(|| anyhow!("failed to parse /proc/meminfo"))
}

/// Live disk usage of the filesystem backing the backups/data directory, via
/// `df -B1`: `(used_bytes, total_bytes)`. Shelling out (rather than adding a
/// `statvfs`-wrapping crate) matches how this codebase already invokes
/// system CLIs for one-off queries (e.g. `docker build`).
pub async fn disk_usage() -> Result<(i64, i64)> {
    let dir = config::get_backups_dir().map_err(|e| anyhow!("{e}"))?;
    let output = tokio::process::Command::new("df").arg("-B1").arg(&dir).output().await?;
    if !output.status.success() {
        return Err(anyhow!("df exited with {}", output.status));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_df_output(&stdout).ok_or_else(|| anyhow!("unparseable df output: {stdout}"))
}

async fn docker_stats_once(docker: &Docker, container_name: &str) -> Result<Stats> {
    let mut stream = docker.stats(container_name, Some(StatsOptions { stream: false, one_shot: false }));
    match stream.next().await {
        Some(Ok(stats)) => Ok(stats),
        Some(Err(e)) => Err(e.into()),
        None => Err(anyhow!("no stats returned for container '{container_name}'")),
    }
}

async fn app_volume_size(docker: &Docker, app_id: &str) -> Result<i64> {
    let volume_name = crate::volume::get_app_volume_name(app_id);
    let usage = docker.df().await?;
    let size = usage
        .volumes
        .unwrap_or_default()
        .into_iter()
        .find(|v| v.name == volume_name)
        .and_then(|v| v.usage_data)
        .map(|u| u.size)
        .unwrap_or(0);
    Ok(size)
}

/// One 60-second sampling tick: reads server-wide CPU/mem/disk and, for
/// every currently-running app, its container CPU/mem plus (every 10th tick
/// only — data-volume size changes slowly and `docker df` walks the whole
/// volume) its data size, persisting all of it to `metric_sample`. Never
/// panics — a sampling failure for one scope just logs a warning and skips
/// that scope's row for this tick.
pub async fn run_tick(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    docker: &Docker,
    prev_host_cpu: &mut Option<ProcStatCpu>,
    tick_count: u64,
    prev_app_disk: &mut HashMap<String, i64>,
) {
    let ts = chrono::Utc::now().to_rfc3339();
    sample_server(pool, &ts, prev_host_cpu).await;

    let apps = match db::app::get_all(pool).await {
        Ok(apps) => apps,
        Err(e) => {
            warn!("metrics: failed to list apps for sampling: {e:#}");
            return;
        }
    };

    for app in apps {
        sample_app(pool, docker, &app, &ts, tick_count, prev_app_disk).await;
    }
}

async fn sample_server(pool: &sqlx::Pool<sqlx::Sqlite>, ts: &str, prev_host_cpu: &mut Option<ProcStatCpu>) {
    let cpu_pct = match tokio::fs::read_to_string("/proc/stat").await {
        Ok(contents) => match parse_proc_stat_cpu_line(&contents) {
            Some(curr) => {
                let pct = prev_host_cpu.as_ref().and_then(|prev| cpu_pct_from_proc_stat(prev, &curr));
                *prev_host_cpu = Some(curr);
                pct
            }
            None => {
                warn!("metrics: failed to parse /proc/stat");
                None
            }
        },
        Err(e) => {
            warn!("metrics: failed to read /proc/stat: {e}");
            None
        }
    };

    let mem_bytes = mem_usage().await.map(|(used, _)| used).ok();
    let disk_bytes = disk_usage().await.map(|(used, _)| used).ok();

    if let Err(e) = db::metrics::insert_sample(pool, ts, "server", cpu_pct, mem_bytes, disk_bytes).await {
        warn!("metrics: failed to save server sample: {e:#}");
    }
}

async fn sample_app(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    docker: &Docker,
    app: &crate::models::App,
    ts: &str,
    tick_count: u64,
    prev_app_disk: &mut HashMap<String, i64>,
) {
    let live = crate::docker::live_state(&app.name).await.unwrap_or(app.state);
    if live != crate::models::AppState::Running {
        return;
    }

    let container_name = format!("{}-container", app.name);
    let (cpu_pct, mem_bytes) = match docker_stats_once(docker, &container_name).await {
        Ok(stats) => (
            cpu_pct_from_docker_stats(&stats.cpu_stats, &stats.precpu_stats),
            stats.memory_stats.usage.map(|u| u as i64),
        ),
        Err(e) => {
            warn!("metrics: failed to sample container stats for '{}': {e:#}", app.name);
            (None, None)
        }
    };

    let disk_bytes = if tick_count % 10 == 0 {
        match app_volume_size(docker, &app.id).await {
            Ok(size) => {
                prev_app_disk.insert(app.id.clone(), size);
                Some(size)
            }
            Err(e) => {
                warn!("metrics: failed to measure data volume for '{}': {e:#}", app.name);
                prev_app_disk.get(&app.id).copied()
            }
        }
    } else {
        prev_app_disk.get(&app.id).copied()
    };

    if let Err(e) = db::metrics::insert_sample(pool, ts, &app.id, cpu_pct, mem_bytes, disk_bytes).await {
        warn!("metrics: failed to save sample for app '{}': {e:#}", app.name);
    }
}

/// Roll the most recently completed UTC hour's samples into `metric_hourly`,
/// then prune samples older than 24h and hourly rows older than 30 days.
/// Called once per hour from the sampler loop in `commands::server::execute`.
pub async fn rollup_and_prune(pool: &sqlx::Pool<sqlx::Sqlite>) {
    let now = chrono::Utc::now();
    let hour_end = now
        .date_naive()
        .and_hms_opt(now.hour(), 0, 0)
        .expect("hour/0/0 is always a valid time")
        .and_utc();
    let hour_start = hour_end - chrono::Duration::hours(1);
    let hour_start_s = hour_start.to_rfc3339();
    let hour_end_s = hour_end.to_rfc3339();

    if let Err(e) = db::metrics::rollup_hour(pool, &hour_start_s, &hour_end_s).await {
        warn!("metrics: hourly rollup failed: {e:#}");
    }

    let sample_cutoff = (now - chrono::Duration::hours(24)).to_rfc3339();
    if let Err(e) = db::metrics::prune_samples_older_than(pool, &sample_cutoff).await {
        warn!("metrics: sample prune failed: {e:#}");
    }

    let hourly_cutoff = (now - chrono::Duration::days(30)).to_rfc3339();
    if let Err(e) = db::metrics::prune_hourly_older_than(pool, &hourly_cutoff).await {
        warn!("metrics: hourly prune failed: {e:#}");
    }
}
```

- [ ] **Step 2: Build**

```bash
cd /Users/dan/projects/litehouse && cargo build 2>&1 | tail -50
```

Expected: compiles cleanly. Fix any import-path mismatches against your actual `src/lib.rs` module names (e.g. if `docker` or `volume` are re-exported differently) before moving on.

- [ ] **Step 3: Run the full metrics test suite (unchanged pure tests must still pass)**

```bash
cargo test metrics:: -- --nocapture
```

Expected: same 10 tests from Task 6 still pass (this task adds no new unit tests — the added code is thin I/O glue, exercised in Task 9's manual verification instead).

- [ ] **Step 4: Commit**

```bash
git add src/metrics.rs
git commit -m "feat(metrics): add sampler tick and hourly rollup/prune drivers"
```

---

### Task 8: Wire the sampler into `lh serve`

**Files:**
- Modify: `src/commands/server.rs`

- [ ] **Step 1: Add the sampler task**

In `src/commands/server.rs`, add `use crate::metrics;` near the other `use` statements at the top, and add this new block immediately after the existing daily-backup `tokio::spawn` block (i.e. after its closing `}` around line 131, before the `// Build combined router` comment):

```rust
    // Resource-usage sampler: every 60s, snapshot host + per-running-app
    // CPU/mem/disk into `metric_sample`; once an hour, roll the completed
    // hour up into `metric_hourly` and prune old rows. See src/metrics.rs.
    {
        let pool = pool.clone();
        let docker_conn = docker_conn.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            let mut prev_host_cpu = None;
            let mut prev_app_disk = std::collections::HashMap::new();
            let mut tick_count: u64 = 0;
            loop {
                interval.tick().await;
                metrics::run_tick(&pool, &docker_conn, &mut prev_host_cpu, tick_count, &mut prev_app_disk).await;
                if tick_count > 0 && tick_count % 60 == 0 {
                    metrics::rollup_and_prune(&pool).await;
                }
                tick_count += 1;
            }
        });
    }
```

- [ ] **Step 2: Build**

```bash
cd /Users/dan/projects/litehouse && cargo build 2>&1 | tail -30
```

Expected: compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/commands/server.rs
git commit -m "feat(server): spawn the resource-usage sampler alongside the backup scheduler"
```

---

### Task 9: Wire the backup catalog into `run_backup`

**Files:**
- Modify: `src/backup.rs`

- [ ] **Step 1: Thread `pool` into `prune_old_backups` and delete matching catalog rows**

Replace:

```rust
/// Prune old backups under `prefix`, keeping the newest `RETENTION_COUNT`.
#[instrument(skip(client))]
async fn prune_old_backups(client: &S3Client, bucket: &str, prefix: &str) -> Result<()> {
    let keys = list_keys(client, bucket, prefix).await?;
    let doomed = keys_to_prune(&keys, RETENTION_COUNT);
    if !doomed.is_empty() {
        info!(
            "pruning {} old backup(s) under s3://{bucket}/{prefix}",
            doomed.len()
        );
        delete_keys(client, bucket, &doomed).await?;
    }
    Ok(())
}
```

with:

```rust
/// Prune old backups under `prefix`, keeping the newest `RETENTION_COUNT`,
/// and remove the matching rows from the `backup` catalog so it never lists
/// an artifact that no longer exists in S3.
#[instrument(skip(pool, client))]
async fn prune_old_backups(pool: &Pool<Sqlite>, client: &S3Client, bucket: &str, prefix: &str) -> Result<()> {
    let keys = list_keys(client, bucket, prefix).await?;
    let doomed = keys_to_prune(&keys, RETENTION_COUNT);
    if !doomed.is_empty() {
        info!(
            "pruning {} old backup(s) under s3://{bucket}/{prefix}",
            doomed.len()
        );
        delete_keys(client, bucket, &doomed).await?;
        if let Err(e) = db::backup::delete_by_keys(pool, &doomed).await {
            warn!("failed to prune backup catalog rows for s3://{bucket}/{prefix}: {e:#}");
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Record the state-DB snapshot in the catalog**

In `backup_state_db`, replace:

```rust
    let key = state_backup_key(prefix, date);
    upload_file(client, bucket, &key, &snapshot_path).await?;
    let _ = std::fs::remove_file(&snapshot_path);

    prune_old_backups(client, bucket, &state_prefix_root(prefix)).await?;
    Ok(())
}
```

with:

```rust
    let key = state_backup_key(prefix, date);
    upload_file(client, bucket, &key, &snapshot_path).await?;
    let size_bytes = std::fs::metadata(&snapshot_path).map(|m| m.len() as i64).unwrap_or(0);
    let _ = std::fs::remove_file(&snapshot_path);

    if let Err(e) = db::backup::record_upload(pool, "litehouse-state", &key, size_bytes).await {
        warn!("failed to catalog litehouse state backup: {e:#}");
    }

    prune_old_backups(pool, client, bucket, &state_prefix_root(prefix)).await?;
    Ok(())
}
```

- [ ] **Step 3: Record each app's backup in the catalog**

Replace the `backup_app` signature and its upload/prune section — from:

```rust
async fn backup_app(
    docker: &Docker,
    client: &S3Client,
    bucket: &str,
    prefix: Option<&str>,
    date: &str,
    backups_dir: &Path,
    app_id: &str,
    app_name: &str,
) -> Result<()> {
    run_snapshot_container(docker, app_id, app_name, backups_dir).await?;

    let staged_dir = backups_dir.join(app_name);
    if !staged_dir.exists() {
        bail!(
            "snapshot container reported success but staged dir {} is missing",
            staged_dir.display()
        );
    }

    let tarball_path = backups_dir.join(format!("{app_name}-{date}.tar.gz"));
    tar_staged_dir(&staged_dir, &tarball_path)?;

    let key = app_backup_key(prefix, app_name, date);
    let upload_result = upload_file(client, bucket, &key, &tarball_path).await;

    // Clean up local staging regardless of upload outcome.
    let _ = std::fs::remove_file(&tarball_path);
    let _ = std::fs::remove_dir_all(&staged_dir);

    upload_result?;

    prune_old_backups(client, bucket, &app_prefix_root(prefix, app_name)).await?;
    Ok(())
}
```

to:

```rust
async fn backup_app(
    pool: &Pool<Sqlite>,
    docker: &Docker,
    client: &S3Client,
    bucket: &str,
    prefix: Option<&str>,
    date: &str,
    backups_dir: &Path,
    app_id: &str,
    app_name: &str,
) -> Result<()> {
    run_snapshot_container(docker, app_id, app_name, backups_dir).await?;

    let staged_dir = backups_dir.join(app_name);
    if !staged_dir.exists() {
        bail!(
            "snapshot container reported success but staged dir {} is missing",
            staged_dir.display()
        );
    }

    let tarball_path = backups_dir.join(format!("{app_name}-{date}.tar.gz"));
    tar_staged_dir(&staged_dir, &tarball_path)?;

    let key = app_backup_key(prefix, app_name, date);
    let upload_result = upload_file(client, bucket, &key, &tarball_path).await;
    let size_bytes = std::fs::metadata(&tarball_path).map(|m| m.len() as i64).unwrap_or(0);

    // Clean up local staging regardless of upload outcome.
    let _ = std::fs::remove_file(&tarball_path);
    let _ = std::fs::remove_dir_all(&staged_dir);

    upload_result?;

    if let Err(e) = db::backup::record_upload(pool, app_name, &key, size_bytes).await {
        warn!("failed to catalog backup for app '{app_name}': {e:#}");
    }

    prune_old_backups(pool, client, bucket, &app_prefix_root(prefix, app_name)).await?;
    Ok(())
}
```

- [ ] **Step 4: Update the two call sites in `run_backup`**

Replace:

```rust
    match backup_state_db(pool, &client, &bucket, prefix.as_deref(), &date, &backups_dir).await {
```

(this one is unchanged — `backup_state_db` already took `pool`).

Replace:

```rust
        match backup_app(docker, &client, &bucket, prefix.as_deref(), &date, &backups_dir, &app.id, &app.name)
            .await
        {
```

with:

```rust
        match backup_app(pool, docker, &client, &bucket, prefix.as_deref(), &date, &backups_dir, &app.id, &app.name)
            .await
        {
```

- [ ] **Step 5: Build**

```bash
cd /Users/dan/projects/litehouse && cargo build 2>&1 | tail -50
```

Expected: compiles cleanly.

- [ ] **Step 6: Run the existing backup test suite to confirm nothing broke**

```bash
cargo test backup:: -- --nocapture
```

Expected: all pre-existing `src/backup.rs` tests still pass (pure `keys_to_prune`/`newest_key`/key-format tests are untouched by this task).

- [ ] **Step 7: Commit**

```bash
git add src/backup.rs
git commit -m "feat(backup): catalog every successful upload and prune catalog rows alongside S3"
```

---

### Task 10: `GET /backups` page

**Files:**
- Create: `templates/backups.html`
- Modify: `src/ui.rs`

- [ ] **Step 1: Write the template**

`templates/backups.html`:

```html
{% extends "base.html" %}

{% block title %}backups - litehouse{% endblock %}

{% block content %}
<p><a href="/">&larr; all apps</a></p>

<h2>Backups</h2>

{% if backups.is_empty() %}
<div class="card">
  <span class="panel-label">catalog</span>
  <p class="muted">No backups recorded yet — the catalog fills in as backups run.</p>
</div>
{% else %}
<table>
  <thead>
    <tr>
      <th>App</th>
      <th>Date</th>
      <th>Size</th>
      <th>Age</th>
    </tr>
  </thead>
  <tbody>
    {% for b in backups %}
    <tr>
      <td>{{ b.app_name }}</td>
      <td>{{ b.s3_key }}</td>
      <td>{{ b.size }}</td>
      <td>{{ b.age }}</td>
    </tr>
    {% endfor %}
  </tbody>
</table>
{% endif %}
{% endblock %}
```

- [ ] **Step 2: Add the template struct, route, and handler**

In `src/ui.rs`, add a new struct near the other template structs (after `AppDetailTemplate`):

```rust
struct BackupRow {
    app_name: String,
    s3_key: String,
    size: String,
    age: String,
}

#[derive(Template)]
#[template(path = "backups.html")]
struct BackupsTemplate {
    backups: Vec<BackupRow>,
}
```

Add the route in `create_ui_router`, inside the `protected` router (alongside `.route("/backup/run", post(run_backup_ui))`):

```rust
        .route("/backups", get(backups_page))
```

Add the handler, near `apps_index`:

```rust
async fn backups_page(State(state): State<Arc<RwLock<AppState>>>) -> Response {
    let pool = state.read().await.db_pool.clone();
    let records = match db::backup::list_all(&pool).await {
        Ok(records) => records,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to list backups: {e}"),
            )
                .into_response();
        }
    };

    let backups = records
        .into_iter()
        .map(|b| BackupRow {
            app_name: b.app_name,
            s3_key: b.s3_key,
            size: chart::format_bytes(b.size_bytes),
            age: relative_time(&b.created_at),
        })
        .collect();

    HtmlTemplate(BackupsTemplate { backups }).into_response()
}
```

- [ ] **Step 3: Add a link from the index backups card**

In `templates/apps.html`, inside the existing backups `<div class="card">` block, change:

```html
    <span class="muted">{{ backup_line }}</span>
    <form class="inline" method="post" action="/backup/run">
      <button type="submit">run now</button>
    </form>
```

to:

```html
    <span class="muted">{{ backup_line }}</span>
    <a href="/backups">view all</a>
    <form class="inline" method="post" action="/backup/run">
      <button type="submit">run now</button>
    </form>
```

- [ ] **Step 4: Build**

```bash
cd /Users/dan/projects/litehouse && cargo build 2>&1 | tail -50
```

Expected: compiles cleanly.

- [ ] **Step 5: Write and run router tests**

Add to the `#[cfg(test)] mod tests` block in `src/ui.rs`:

```rust
    #[tokio::test]
    async fn backups_page_without_cookie_redirects_to_login() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/backups")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/login");
    }

    #[tokio::test]
    async fn backups_page_shows_empty_state_with_no_catalog_rows() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/backups")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("No backups recorded yet"));
    }

    #[tokio::test]
    async fn backups_page_lists_catalog_rows() {
        let state = test_state().await;
        {
            let s = state.read().await;
            db::backup::record_upload(&s.db_pool, "demo-app", "apps/demo-app/2026-07-11.tar.gz", 123456)
                .await
                .unwrap();
        }
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/backups")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("demo-app"));
        assert!(body.contains("2026-07-11.tar.gz"));
        assert!(body.contains("120.6 KB"));
    }
```

```bash
cargo test ui:: -- --nocapture
```

Expected: all `ui.rs` tests (existing + 3 new) pass.

- [ ] **Step 6: Commit**

```bash
git add templates/backups.html templates/apps.html src/ui.rs
git commit -m "feat(ui): add /backups page listing the backup catalog"
```

---

### Task 11: Server resources card on the index page

**Files:**
- Modify: `src/ui.rs`
- Modify: `templates/apps.html`

- [ ] **Step 1: Extend `AppsTemplate` and `apps_index`**

In `src/ui.rs`, add fields to `AppsTemplate`:

```rust
#[derive(Template)]
#[template(path = "apps.html")]
struct AppsTemplate {
    apps: Vec<AppRow>,
    backup_line: String,
    backup_failures: Vec<(String, String)>,
    flash: Option<String>,
    any_in_progress: bool,
    server_cpu_chart: String,
    server_mem_chart: String,
    server_mem_total: String,
    server_disk_chart: String,
    server_disk_total: String,
}
```

Add a helper function near `apps_index`:

```rust
async fn build_server_metrics_charts(pool: &sqlx::Pool<sqlx::Sqlite>) -> (String, String, String, String, String) {
    let since = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
    let rows = db::metrics::list_samples_since(pool, "server", &since).await.unwrap_or_default();
    let cpu: Vec<Option<f64>> = rows.iter().map(|r| r.cpu_pct).collect();
    let mem: Vec<Option<f64>> = rows.iter().map(|r| r.mem_bytes.map(|v| v as f64)).collect();
    let disk: Vec<Option<f64>> = rows.iter().map(|r| r.disk_bytes.map(|v| v as f64)).collect();

    let mem_total = match crate::metrics::mem_usage().await {
        Ok((_, total)) => chart::format_bytes(total),
        Err(_) => "unknown".to_string(),
    };
    let disk_total = match crate::metrics::disk_usage().await {
        Ok((_, total)) => chart::format_bytes(total),
        Err(_) => "unknown".to_string(),
    };

    (
        chart::line_chart(&cpu, chart::ChartUnit::Percent),
        chart::line_chart(&mem, chart::ChartUnit::Bytes),
        mem_total,
        chart::line_chart(&disk, chart::ChartUnit::Bytes),
        disk_total,
    )
}
```

In `apps_index`, after the `let (backup_line, backup_failures) = ...` block and before `HtmlTemplate(AppsTemplate { ... })`, add:

```rust
    let (server_cpu_chart, server_mem_chart, server_mem_total, server_disk_chart, server_disk_total) =
        build_server_metrics_charts(&pool).await;
```

Update the final `HtmlTemplate(AppsTemplate { ... })` construction to include the five new fields (`server_cpu_chart`, `server_mem_chart`, `server_mem_total`, `server_disk_chart`, `server_disk_total`) alongside the existing ones.

- [ ] **Step 2: Add the card to the template**

In `templates/apps.html`, insert this new card immediately after the existing backups `<div class="card">...</div>` block and before the `{% if apps.is_empty() %}` line:

```html
<div class="card">
  <span class="panel-label">server resources</span>
  <div class="metrics-grid">
    <div>
      <h4>CPU <span class="muted">24h</span></h4>
      {{ server_cpu_chart|safe }}
    </div>
    <div>
      <h4>Memory <span class="muted">of {{ server_mem_total }}</span></h4>
      {{ server_mem_chart|safe }}
    </div>
    <div>
      <h4>Disk <span class="muted">of {{ server_disk_total }}</span></h4>
      {{ server_disk_chart|safe }}
    </div>
  </div>
</div>
```

- [ ] **Step 3: Add CSS for the chart layout**

In `src/ui/styles.css`, append at the end of the file:

```css
/* ---------------------------------------------------------------------
   Resource charts
   --------------------------------------------------------------------- */

.metrics-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 1.25rem;
  margin-top: 0.5rem;
}

.metrics-grid h4 {
  margin: 0 0 0.35rem;
  font-size: 0.8rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--fg);
}

.chart {
  color: var(--accent);
}

.chart-svg {
  display: block;
  width: 100%;
  height: 70px;
}

.chart-label {
  display: block;
  margin-top: 0.25rem;
  font-size: 0.8rem;
  color: var(--muted);
}

.chart-empty {
  height: 70px;
  display: flex;
  align-items: center;
}
```

- [ ] **Step 4: Build**

```bash
cd /Users/dan/projects/litehouse && cargo build 2>&1 | tail -50
```

Expected: compiles cleanly.

- [ ] **Step 5: Add a router test for the server resources card**

Add to `src/ui.rs`'s test module:

```rust
    #[tokio::test]
    async fn index_shows_server_resources_card_with_no_samples() {
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

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("server resources"));
        // No samples in a fresh test DB -> both charts render the empty state.
        assert!(body.contains("no data yet"));
    }

    #[tokio::test]
    async fn index_server_resources_card_renders_chart_from_samples() {
        let state = test_state().await;
        {
            let s = state.read().await;
            db::metrics::insert_sample(&s.db_pool, "2026-07-12T10:00:00+00:00", "server", Some(12.0), Some(1_000_000), Some(2_000_000))
                .await
                .unwrap();
            db::metrics::insert_sample(&s.db_pool, "2026-07-12T10:01:00+00:00", "server", Some(18.0), Some(1_100_000), Some(2_000_000))
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
        assert!(body.contains("<polyline"));
        assert!(body.contains("18.0%"));
    }
```

```bash
cargo test ui:: -- --nocapture
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/ui.rs templates/apps.html src/ui/styles.css
git commit -m "feat(ui): add server resources card with 24h CPU/mem/disk charts to the index"
```

---

### Task 12: Per-app metrics card on the app detail page

**Files:**
- Modify: `src/ui.rs`
- Modify: `templates/app_detail.html`

- [ ] **Step 1: Extend the query struct and `AppDetailTemplate`**

In `src/ui.rs`, replace the existing `FlashQuery` usage on the app detail route with a dedicated query struct (leave `FlashQuery` itself as-is — it's still used by `apps_index`). Add near `FlashQuery`:

```rust
#[derive(Debug, Deserialize)]
struct AppDetailQuery {
    flash: Option<String>,
    range: Option<String>,
}
```

Add fields to `AppDetailTemplate`:

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
    metrics_range: String,
    cpu_chart: String,
    mem_chart: String,
    disk_chart: String,
}
```

- [ ] **Step 2: Add the chart-building helper and wire it into `app_detail`**

Add near `app_detail`:

```rust
async fn build_app_metrics_charts(pool: &sqlx::Pool<sqlx::Sqlite>, scope: &str, range: &str) -> (String, String, String) {
    if range == "30d" {
        let since = (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339();
        let rows = db::metrics::list_hourly_since(pool, scope, &since).await.unwrap_or_default();
        let cpu_avg: Vec<Option<f64>> = rows.iter().map(|r| r.cpu_avg).collect();
        let cpu_min: Vec<Option<f64>> = rows.iter().map(|r| r.cpu_min).collect();
        let cpu_max: Vec<Option<f64>> = rows.iter().map(|r| r.cpu_max).collect();
        let mem_avg: Vec<Option<f64>> = rows.iter().map(|r| r.mem_avg.map(|v| v as f64)).collect();
        let mem_min: Vec<Option<f64>> = rows.iter().map(|r| r.mem_min.map(|v| v as f64)).collect();
        let mem_max: Vec<Option<f64>> = rows.iter().map(|r| r.mem_max.map(|v| v as f64)).collect();
        let disk_avg: Vec<Option<f64>> = rows.iter().map(|r| r.disk_avg.map(|v| v as f64)).collect();
        let disk_min: Vec<Option<f64>> = rows.iter().map(|r| r.disk_min.map(|v| v as f64)).collect();
        let disk_max: Vec<Option<f64>> = rows.iter().map(|r| r.disk_max.map(|v| v as f64)).collect();
        (
            chart::band_chart(&cpu_avg, &cpu_min, &cpu_max, chart::ChartUnit::Percent),
            chart::band_chart(&mem_avg, &mem_min, &mem_max, chart::ChartUnit::Bytes),
            chart::band_chart(&disk_avg, &disk_min, &disk_max, chart::ChartUnit::Bytes),
        )
    } else {
        let since = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
        let rows = db::metrics::list_samples_since(pool, scope, &since).await.unwrap_or_default();
        let cpu: Vec<Option<f64>> = rows.iter().map(|r| r.cpu_pct).collect();
        let mem: Vec<Option<f64>> = rows.iter().map(|r| r.mem_bytes.map(|v| v as f64)).collect();
        let disk: Vec<Option<f64>> = rows.iter().map(|r| r.disk_bytes.map(|v| v as f64)).collect();
        (
            chart::line_chart(&cpu, chart::ChartUnit::Percent),
            chart::line_chart(&mem, chart::ChartUnit::Bytes),
            chart::line_chart(&disk, chart::ChartUnit::Bytes),
        )
    }
}
```

In `app_detail`, change the handler signature's query extractor from `Query(q): Query<FlashQuery>` to `Query(q): Query<AppDetailQuery>`, then add (after the `let deploys = ...` block, before `HtmlTemplate(AppDetailTemplate { ... })`):

```rust
    let metrics_range = match q.range.as_deref() {
        Some("30d") => "30d".to_string(),
        _ => "24h".to_string(),
    };
    let (cpu_chart, mem_chart, disk_chart) = build_app_metrics_charts(&pool, &app.id, &metrics_range).await;
```

Update the `HtmlTemplate(AppDetailTemplate { ... })` construction to include `metrics_range`, `cpu_chart`, `mem_chart`, `disk_chart` alongside the existing fields.

- [ ] **Step 3: Add the card to the template**

In `templates/app_detail.html`, insert this block after the closing `</div>` of `.detail-grid` and before the `<h3>Deploys</h3>` line:

```html
<h3>Resources</h3>
<p class="inline-links">
  <a href="/apps/{{ app_name }}?range=24h"{% if metrics_range == "24h" %} class="active"{% endif %}>24h</a>
  &middot;
  <a href="/apps/{{ app_name }}?range=30d"{% if metrics_range == "30d" %} class="active"{% endif %}>30d</a>
</p>
<div class="metrics-grid">
  <div><h4>CPU</h4>{{ cpu_chart|safe }}</div>
  <div><h4>Memory</h4>{{ mem_chart|safe }}</div>
  <div><h4>Data size</h4>{{ disk_chart|safe }}</div>
</div>
```

- [ ] **Step 4: Add CSS for the range toggle**

Append to `src/ui/styles.css` (after the metrics-grid rules added in Task 11):

```css
.inline-links a {
  margin-right: 0.25rem;
}

.inline-links a.active {
  color: var(--fg);
  text-decoration: none;
  font-weight: 700;
}
```

- [ ] **Step 5: Build**

```bash
cd /Users/dan/projects/litehouse && cargo build 2>&1 | tail -50
```

Expected: compiles cleanly.

- [ ] **Step 6: Add router tests**

Add to `src/ui.rs`'s test module:

```rust
    #[tokio::test]
    async fn app_detail_shows_resources_card_with_no_samples() {
        let state = state_with_app("metrics-app").await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/apps/metrics-app")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("Resources"));
        assert!(body.contains("no data yet"));
    }

    #[tokio::test]
    async fn app_detail_range_toggle_switches_to_hourly_rollups() {
        let state = state_with_app("metrics-app-30d").await;
        {
            let s = state.read().await;
            let app_row = db::app::get_by_name(&s.db_pool, "metrics-app-30d").await.unwrap().unwrap();
            db::metrics::insert_sample(&s.db_pool, "2026-07-12T10:00:00+00:00", &app_row.id, Some(5.0), Some(1000), Some(2000))
                .await
                .unwrap();
            db::metrics::rollup_hour(&s.db_pool, "2026-07-12T10:00:00+00:00", "2026-07-12T11:00:00+00:00")
                .await
                .unwrap();
        }
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/apps/metrics-app-30d?range=30d")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("5.0% avg"));
    }
```

These reference a `state_with_app` helper that doesn't exist yet in the test module — add it near `test_state`:

```rust
    async fn state_with_app(name: &str) -> Arc<RwLock<AppState>> {
        let state = test_state().await;
        {
            let s = state.read().await;
            let app = App::new(name).unwrap();
            db::app::save(&s.db_pool, &app).await.unwrap();
        }
        state
    }
```

```bash
cargo test ui:: -- --nocapture
```

Expected: all tests pass (existing + these 2 new + the `state_with_app` helper compiles).

- [ ] **Step 7: Commit**

```bash
git add src/ui.rs templates/app_detail.html src/ui/styles.css
git commit -m "feat(ui): add per-app CPU/mem/data-size metrics card with 24h/30d toggle"
```

---

### Task 13: Full test suite + manual verification

**Files:** none (verification only)

- [ ] **Step 1: Run the full test suite**

```bash
cd /Users/dan/projects/litehouse && cargo test 2>&1 | tail -80
```

Expected: all tests pass, including every test added in Tasks 1–12.

- [ ] **Step 2: Regenerate the sqlx cache one final time and confirm it's in sync**

```bash
DATABASE_URL=sqlite://config/dev.db cargo sqlx prepare -- --all-targets
git status --porcelain .sqlx
```

Expected: no changes reported (the cache was already up to date from earlier tasks) — if there are changes, stage and commit them.

- [ ] **Step 3: Manually verify the sampler against a real Docker daemon**

```bash
rm -f config/dev.db*
LITEHOUSE_LOCAL_DEV=1 DATABASE_URL=sqlite://config/dev.db cargo run -- serve &
sleep 90
sqlite3 config/dev.db "SELECT ts, scope, cpu_pct, mem_bytes, disk_bytes FROM metric_sample ORDER BY ts DESC LIMIT 5;"
```

Expected: at least one `server`-scope row with a non-null `cpu_pct` and `mem_bytes` (the very first tick's `cpu_pct` will be `NULL` — there's no previous reading yet — so wait for the second tick, ~90s in). Stop the server afterward (`kill %1`).

- [ ] **Step 4: Manually verify the UI in a browser**

With the server still running (or restarted per Step 3), open `http://admin.localhost:9090` (per the local-dev URL convention in `src/ui.rs::app_url`), log in with the admin token printed on server startup, and confirm:
- The index page shows a "server resources" card with three charts (they'll show "no data yet" until ~2 sampler ticks have landed).
- The backups card has a "view all" link to `/backups`, which renders (empty state is fine if no backup has run).
- An app detail page shows a "Resources" section with a 24h/30d toggle; clicking "30d" doesn't error (empty state is fine with no data yet).

Report any visual issues before considering this task done — this is UI work, so running it in a browser is required, not optional (per this repo's own contribution guidance).

- [ ] **Step 5: Clean up the scratch dev DB**

```bash
rm -f /Users/dan/projects/litehouse/config/dev.db*
```

(`config/dev.db` was only a throwaway DB for `cargo sqlx prepare`/manual testing — never committed; confirm `.gitignore` already excludes `config/*.db` or add it if not.)

```bash
grep -q "config/\*.db" /Users/dan/projects/litehouse/.gitignore || echo "config/*.db*" >> /Users/dan/projects/litehouse/.gitignore
git add .gitignore
git status --porcelain
```

If `.gitignore` was changed, commit it:

```bash
git commit -m "chore: ignore scratch dev DB used for sqlx offline cache generation"
```

---

## Plan self-review notes

- **Spec coverage:** backup catalog (Task 4, 9, 10), server resource tracking (Task 11), per-app resource tracking with 24h/30d toggle (Task 12), SVG-only charts with no new JS (Task 5) — all covered.
- **Corrected inconsistency:** added `cpu_min`/`mem_min`/`disk_min` to `metric_hourly` (the design doc's schema sketch omitted these, but the "min/max band" requirement needs them) — flagged at the top of this plan.
- **Type consistency check:** `MetricHourly`/`MetricSample` field names match between `src/models/metric.rs` (Task 2), the `sqlx::query_as!` column lists in `src/db/metrics.rs` (Task 3), and every place they're destructured in `src/ui.rs` (Tasks 11–12). `BackupRecord` fields match between Task 2, Task 4's queries, and Task 10's template mapping.
