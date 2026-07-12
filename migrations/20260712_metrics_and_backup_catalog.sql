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
