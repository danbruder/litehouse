use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::{now, UtcDateTime};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemConfig {
    pub id: String,
    pub config_type: String,

    // S3 Configuration (optional fields)
    pub s3_access_key_id: Option<String>,
    pub s3_secret_access_key: Option<String>,
    pub s3_bucket: Option<String>,
    pub s3_region: Option<String>,
    pub s3_endpoint: Option<String>,
    pub s3_path_prefix: Option<String>,

    // GHCR token configuration (optional field)
    pub ghcr_token: Option<String>,

    pub created_at: UtcDateTime,
    pub updated_at: UtcDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Config {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub path_prefix: Option<String>,
}

impl SystemConfig {
    /// Create a new S3 backup configuration
    pub fn new_s3_config(s3_config: &S3Config) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            config_type: "s3_backup".to_string(),
            s3_access_key_id: Some(s3_config.access_key_id.clone()),
            s3_secret_access_key: Some(s3_config.secret_access_key.clone()),
            s3_bucket: Some(s3_config.bucket.clone()),
            s3_region: Some(s3_config.region.clone()),
            s3_endpoint: s3_config.endpoint.clone(),
            s3_path_prefix: s3_config.path_prefix.clone().or(Some("litehouse".to_string())),
            ghcr_token: None,
            created_at: now(),
            updated_at: now(),
        }
    }

    /// Create a new GHCR token configuration
    pub fn new_ghcr_token(token: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            config_type: "ghcr_token".to_string(),
            s3_access_key_id: None,
            s3_secret_access_key: None,
            s3_bucket: None,
            s3_region: None,
            s3_endpoint: None,
            s3_path_prefix: None,
            ghcr_token: Some(token.to_string()),
            created_at: now(),
            updated_at: now(),
        }
    }

    /// Convert SystemConfig to S3Config (returns None if not S3 type or missing required fields)
    pub fn to_s3_config(&self) -> Option<S3Config> {
        if self.config_type != "s3_backup" {
            return None;
        }

        Some(S3Config {
            access_key_id: self.s3_access_key_id.as_ref()?.clone(),
            secret_access_key: self.s3_secret_access_key.as_ref()?.clone(),
            bucket: self.s3_bucket.as_ref()?.clone(),
            region: self.s3_region.as_ref()?.clone(),
            endpoint: self.s3_endpoint.clone(),
            path_prefix: self.s3_path_prefix.clone(),
        })
    }
}

/// Redacted version of S3Config for API responses (hides secret key)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3ConfigRedacted {
    pub access_key_id: String,
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub path_prefix: Option<String>,
}

impl From<&S3Config> for S3ConfigRedacted {
    fn from(config: &S3Config) -> Self {
        Self {
            access_key_id: config.access_key_id.clone(),
            bucket: config.bucket.clone(),
            region: config.region.clone(),
            endpoint: config.endpoint.clone(),
            path_prefix: config.path_prefix.clone(),
        }
    }
}
