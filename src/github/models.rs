use serde::{Deserialize, Serialize};

/// Response from GitHub's device authorization endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceAuthResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// Response from GitHub's token endpoint
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TokenResponse {
    Success {
        access_token: String,
        token_type: String,
        scope: String,
    },
    Pending {
        error: String,
        error_description: Option<String>,
    },
}

/// GitHub user profile
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitHubUser {
    pub id: i64,
    pub login: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

/// GitHub repository
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Repository {
    pub id: i64,
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    pub private: bool,
    pub html_url: String,
    pub clone_url: String,
    pub ssh_url: String,
    pub default_branch: String,
    pub updated_at: String,
    pub pushed_at: Option<String>,
}

/// GitHub repository search response
#[derive(Debug, Clone, Deserialize)]
pub struct SearchResponse {
    pub total_count: u32,
    pub incomplete_results: bool,
    pub items: Vec<Repository>,
}

/// Simplified repository info for CLI display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoInfo {
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    pub private: bool,
    pub clone_url: String,
    pub default_branch: String,
    pub updated_at: String,
}

impl From<Repository> for RepoInfo {
    fn from(repo: Repository) -> Self {
        Self {
            name: repo.name,
            full_name: repo.full_name,
            description: repo.description,
            private: repo.private,
            clone_url: repo.clone_url,
            default_branch: repo.default_branch,
            updated_at: repo.updated_at,
        }
    }
}
