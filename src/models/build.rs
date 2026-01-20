use crate::models::{now, UtcDateTime};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct Build {
    pub id: String,
    pub app_id: String,
    pub image_id: String,
    pub image_tag: String,
    pub git_commit: String,
    pub log_path: Option<String>,
    pub created_at: UtcDateTime,
    pub updated_at: UtcDateTime,
}

#[derive(Debug)]
pub struct BuildInput {
    pub app_id: String,
    pub image_id: String,
    pub image_tag: String,
    pub git_commit: String,
    pub log_path: Option<String>,
}

impl BuildInput {
    pub fn new(app_id: String, image_id: String, image_tag: String, git_commit: String) -> BuildInput {
        BuildInput { app_id, image_id, image_tag, git_commit, log_path: None }
    }

    pub fn with_log_path(mut self, log_path: String) -> BuildInput {
        self.log_path = Some(log_path);
        self
    }
}

impl Build {
    pub fn new(input: BuildInput) -> Build {
        let now = now();
        Build {
            id: Uuid::new_v4().to_string(),
            app_id: input.app_id,
            image_id: input.image_id,
            image_tag: input.image_tag,
            git_commit: input.git_commit,
            log_path: input.log_path,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}
