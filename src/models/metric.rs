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
