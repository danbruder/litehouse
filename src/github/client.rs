use anyhow::{anyhow, Result};
use reqwest::Client;

use super::models::{GitHubUser, Repository, RepoInfo, SearchResponse};

const API_BASE_URL: &str = "https://api.github.com";

pub struct GitHubClient {
    client: Client,
    access_token: String,
}

impl GitHubClient {
    pub fn new(access_token: &str) -> Self {
        Self {
            client: Client::new(),
            access_token: access_token.to_string(),
        }
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.access_token)
    }

    /// Get the authenticated user's profile
    pub async fn get_user(&self) -> Result<GitHubUser> {
        let response = self
            .client
            .get(format!("{}/user", API_BASE_URL))
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "litehouse")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("Failed to get user: {}", error));
        }

        let user: GitHubUser = response.json().await?;
        Ok(user)
    }

    /// List repositories for the authenticated user
    pub async fn list_repos(&self, limit: u32) -> Result<Vec<RepoInfo>> {
        let per_page = limit.min(100);
        let response = self
            .client
            .get(format!(
                "{}/user/repos?sort=updated&direction=desc&per_page={}",
                API_BASE_URL, per_page
            ))
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "litehouse")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("Failed to list repos: {}", error));
        }

        let repos: Vec<Repository> = response.json().await?;
        Ok(repos.into_iter().map(RepoInfo::from).collect())
    }

    /// Search repositories accessible to the authenticated user
    pub async fn search_repos(&self, query: &str) -> Result<Vec<RepoInfo>> {
        let response = self
            .client
            .get(format!(
                "{}/search/repositories?q={}&sort=updated&order=desc&per_page=30",
                API_BASE_URL,
                urlencoding::encode(query)
            ))
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "litehouse")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("Failed to search repos: {}", error));
        }

        let search_result: SearchResponse = response.json().await?;
        Ok(search_result.items.into_iter().map(RepoInfo::from).collect())
    }

    /// Get a specific repository by owner/name
    pub async fn get_repo(&self, owner: &str, name: &str) -> Result<RepoInfo> {
        let response = self
            .client
            .get(format!("{}/repos/{}/{}", API_BASE_URL, owner, name))
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "litehouse")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("Failed to get repo {}/{}: {}", owner, name, error));
        }

        let repo: Repository = response.json().await?;
        Ok(RepoInfo::from(repo))
    }
}
