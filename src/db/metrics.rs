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
