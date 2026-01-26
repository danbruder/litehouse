use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubPushPayload {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub repository: Repository,
    pub head_commit: Option<Commit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub clone_url: String,
    pub ssh_url: String,
    pub html_url: String,
    pub full_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    pub id: String,
    pub message: String,
    pub timestamp: String,
    pub author: Author,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    pub email: String,
}
