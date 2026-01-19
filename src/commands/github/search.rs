use crate::api_client::ApiClient;
use anyhow::Result;

pub async fn execute(api_client: &ApiClient, query: &str) -> Result<()> {
    let repos = api_client.github_search_repos(query).await?;

    if repos.is_empty() {
        println!("No repositories found for '{}'", query);
        return Ok(());
    }

    println!("Found {} repositories:\n", repos.len());
    println!("{:<40} {:<50}", "NAME", "DESCRIPTION");
    println!("{}", "-".repeat(90));

    for repo in repos {
        let description = repo
            .description
            .as_deref()
            .unwrap_or("")
            .chars()
            .take(47)
            .collect::<String>();
        let description = if description.len() == 47 {
            format!("{}...", description)
        } else {
            description
        };

        println!("{:<40} {}", repo.full_name, description);
    }

    Ok(())
}
