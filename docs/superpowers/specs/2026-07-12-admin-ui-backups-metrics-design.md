# Admin UI: Backup Catalog + Resource Metrics — Design

**Date:** 2026-07-12
**Status:** Approved

## Goal

Extend the server-rendered admin UI (`src/ui.rs`, Askama + HTMX) with:

1. **Backup management (list only)** — browse backups from a DB-backed catalog, not by listing S3 on page load.
2. **Server resource tracking over time** — CPU, memory, disk usage of the host.
3. **Per-app resource tracking over time** — CPU, memory, and app data (SQLite volume) size.

Charts are **server-rendered SVG** — no new JS dependencies (HTMX stays the only script). No delete/download/restore actions on backups in this iteration.

## Non-goals

- Backup delete, download, or point-in-time restore from the UI (retention pruning already handles cleanup).
- Alerting, thresholds, or notifications.
- Backfilling the catalog from pre-existing S3 objects — the catalog reflects backups taken after this ships.
- Exposing metrics via the JSON API / CLI (UI-only for now; tables are there if we want it later).

## Component 1: Metrics sampler (`src/metrics.rs`, new module)

A background tokio task spawned by `lh serve` (alongside the existing hourly backup scheduler in `src/commands/server.rs`), ticking every **60 seconds**.

### What is sampled

**Server scope** (litehouse-server runs in a container, but these are host-wide):
- **CPU %**: delta of `/proc/stat` aggregate cpu line between ticks (host-wide; not cgroup-namespaced).
- **Memory**: used bytes from `/proc/meminfo` (`MemTotal - MemAvailable`).
- **Disk**: used bytes + total of the filesystem backing the data/backups dir (`statvfs` on `config::get_backups_dir()`, which is bind-mounted from the host).

**Per-app scope**, for every app whose container is running:
- **CPU %** and **memory bytes**: bollard one-shot container stats (`stats` with `stream: false`), computed the same way `docker stats` does (cpu delta / system delta × online CPUs).
- **Data size**: per-volume usage from bollard's data-usage endpoint (`docker system df -v` equivalent), matched by `volume::get_app_volume_name(app_id)`. Because `df` walks volume contents, it is sampled only every **10th tick** (10 min) and the last value is carried forward on intermediate ticks.

Sampling failures (Docker hiccup, missing `/proc` field) log a warning and skip that tick — the sampler never crashes the server. A missed metric is a `NULL` column, not a missing row, when other metrics for that scope succeeded.

### Storage (state DB, new migration)

```sql
CREATE TABLE metric_sample (
    ts    TEXT NOT NULL,   -- RFC3339 UTC, minute resolution
    scope TEXT NOT NULL,   -- 'server' or app id
    cpu_pct    REAL,
    mem_bytes  INTEGER,
    disk_bytes INTEGER,    -- server: fs used bytes; app: data volume bytes
    PRIMARY KEY (ts, scope)
);

CREATE TABLE metric_hourly (
    hour  TEXT NOT NULL,   -- RFC3339 truncated to the hour, e.g. '2026-07-12T14:00:00Z'
    scope TEXT NOT NULL,
    cpu_avg REAL,    cpu_max REAL,
    mem_avg INTEGER, mem_max INTEGER,
    disk_avg INTEGER, disk_max INTEGER,
    samples INTEGER NOT NULL,
    PRIMARY KEY (hour, scope)
);
```

Server disk *total* and host mem *total* are cheap to read at render time and are not stored per-sample.

### Rollup & retention

Once per hour (on the sampler's own tick counter, no second task):
1. Aggregate completed hours from `metric_sample` into `metric_hourly` (avg/max per scope; idempotent `INSERT OR REPLACE`).
2. Delete `metric_sample` rows older than **24 hours**.
3. Delete `metric_hourly` rows older than **30 days**.

Rollup/pruning logic is pure SQL over the pool → unit-testable against the in-memory test pool with synthetic rows. Deleting an app leaves its historical rows to age out naturally (scope is the app id; no FK).

## Component 2: Backup catalog (`backup` table)

The DB becomes the **catalog**; S3 remains the storage. Same migration adds:

```sql
CREATE TABLE backup (
    id         TEXT PRIMARY KEY,        -- uuid
    app_name   TEXT NOT NULL,           -- app name, or 'litehouse-state' for the state DB snapshot
    s3_key     TEXT NOT NULL UNIQUE,
    size_bytes INTEGER NOT NULL,
    status     TEXT NOT NULL,           -- 'succeeded' (failed attempts stay in the existing report, not the catalog)
    created_at TEXT NOT NULL            -- RFC3339
);
```

`src/backup.rs` changes (new `src/db/backup.rs` for queries):
- After each successful per-app upload in `run_backup`, insert a row (`INSERT OR REPLACE` keyed on `s3_key`, since re-running on the same day overwrites the same key). Size comes from the local tarball before upload.
- Same for the state-DB snapshot upload (`app_name = 'litehouse-state'`).
- `prune_old_backups` deletes the catalog rows for every S3 key it deletes, keeping catalog ≡ S3 going forward.
- `run_backup`'s signature stays; it already has the pool.

## Component 3: SVG chart helper (`src/ui/chart.rs`, new)

A small pure-Rust function set (~100–150 lines), no deps beyond `std`:

- `line_chart(series: &[(f64, Option<f64>)], opts) -> String` — inline `<svg>` with a polyline (gaps for `None`), a filled area variant, min/max/current labels, and a y-axis that starts at 0 for percentages / auto-scales for bytes.
- A `format_bytes` helper (`1.2 GB`) shared with the templates.
- Pure string-in/string-out → straightforward unit tests (path coordinates, gap handling, empty-series placeholder text).

Charts render inside the existing card aesthetic; colors via the existing CSS variables (stroke uses `currentColor`/CSS classes, so `styles.css` themes them).

## Component 4: UI changes (`src/ui.rs`, templates)

### Index (`/`)
- Backups card: unchanged report line + "run now", plus a link to **`/backups`**.
- New **server resources card**: three sparkline-scale SVG charts (CPU %, memory, disk) over the last 24h of raw samples, with current value + total (e.g. `disk 4.1 / 25 GB`) rendered beside each.

### New page: `GET /backups`
- Table over `db::backup::list_all` (newest first): app, date, size (humanized), age (`relative_time`). Grouped or sorted by app name then date desc. Empty state: "No backups recorded yet — the catalog fills in as backups run."
- Protected route, same middleware.

### App detail (`/apps/:name`)
- New **metrics card**: three larger SVG charts — CPU %, memory, data size — with a `?range=24h|30d` toggle (two links, styled as tabs).
  - `24h`: raw `metric_sample` rows for the app's scope.
  - `30d`: `metric_hourly` rows, drawing the avg line with a min/max band (second translucent area path).
- Empty state when no samples exist yet: "Collecting metrics — check back in a few minutes."

No HTMX polling on charts initially (a page reload refreshes them); the existing log-tail/deploy polling patterns can be extended later if it itches.

## Data flow summary

```
sampler (60s tick) ──► metric_sample ──hourly rollup──► metric_hourly
                            │                              │
                            └── index card + 24h range ────┴── 30d range
run_backup ──► S3 upload ──► backup table ──► /backups page
        └──► prune S3 ──► delete matching backup rows
```

## Error handling

- Sampler: warn-and-skip per tick; never panics, never blocks serve startup (spawned task).
- UI queries: follow the existing pattern — `unwrap_or_default()` for decorative data (charts render the empty state), 500 with message only for page-critical queries (the backups list itself).
- Catalog insert failure during backup: log a warning; the backup itself still counts as succeeded (S3 is the source of truth for data safety, the catalog is a view).

## Testing

- **Unit**: rollup/prune SQL against the in-memory pool with synthetic timestamps; `chart::line_chart` path/gap/empty cases; `format_bytes`; CPU-delta math on fixed `/proc/stat` fixtures; catalog insert/replace + prune-sync.
- **Router (oneshot)**, following existing `ui.rs` test patterns: `/backups` renders catalog rows and requires auth; index renders the server-resources card; app detail renders the metrics card and honors `?range=30d`; empty states render when tables are empty.
- Docker-touching sampler internals (bollard stats/df) stay thin and are exercised by the existing `#[ignore]`-style integration approach only if needed — the plan should keep parse/compute logic pure and separately testable.

## Deployment note

No install-script changes required: the migration runs automatically at startup (`sqlx::migrate!`), the sampler starts with `lh serve`, and `/proc` + the backups bind-mount are already visible inside the litehouse-server container. Ship via the standard tag → CI → `dev-deploy.sh` flow.
