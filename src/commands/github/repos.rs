use crate::api_client::ApiClient;
use anyhow::Result;

pub async fn execute(api_client: &ApiClient, limit: u32) -> Result<()> {
    let repos = api_client.github_list_repos(limit).await?;

    if repos.is_empty() {
        println!("No repositories found");
        return Ok(());
    }

    println!("{:<40} {:<50} {}", "NAME", "DESCRIPTION", "UPDATED");
    println!("{}", "-".repeat(100));

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

        // Format the updated_at timestamp
        let updated = format_relative_time(&repo.updated_at);

        println!("{:<40} {:<50} {}", repo.full_name, description, updated);
    }

    Ok(())
}

fn format_relative_time(timestamp: &str) -> String {
    // Parse ISO 8601 timestamp and format as relative time
    // For simplicity, just show the date portion
    if let Some(date_part) = timestamp.split('T').next() {
        date_part.to_string()
    } else {
        timestamp.to_string()
    }
}
