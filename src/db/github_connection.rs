use super::*;
use crate::models::GitHubConnection;

/// Save a GitHub connection to the database (upsert)
#[instrument(skip(pool, connection))]
pub async fn save(pool: &Pool<Sqlite>, connection: &GitHubConnection) -> Result<()> {
    sqlx::query!(
        r#"
            INSERT INTO github_connection (
                id, user_id, github_user_id, github_username, github_email, access_token, scopes, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(user_id) DO UPDATE SET
                github_user_id = excluded.github_user_id,
                github_username = excluded.github_username,
                github_email = excluded.github_email,
                access_token = excluded.access_token,
                scopes = excluded.scopes,
                updated_at = excluded.updated_at
            "#,
        connection.id,
        connection.user_id,
        connection.github_user_id,
        connection.github_username,
        connection.github_email,
        connection.access_token,
        connection.scopes,
        connection.created_at,
        connection.updated_at
    )
    .execute(pool)
    .await?;

    debug!("Saved GitHub connection for user '{}'", connection.user_id);
    Ok(())
}

/// Get a GitHub connection by user ID
#[instrument(skip(pool))]
pub async fn get_by_user_id(pool: &Pool<Sqlite>, user_id: &str) -> Result<Option<GitHubConnection>> {
    let connection = sqlx::query_as!(
        GitHubConnection,
        r#"
            SELECT id, user_id, github_user_id, github_username, github_email, access_token, scopes, created_at, updated_at
            FROM github_connection
            WHERE user_id = ?
            "#,
        user_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(connection)
}

/// Delete a GitHub connection by user ID
#[instrument(skip(pool))]
pub async fn delete_by_user_id(pool: &Pool<Sqlite>, user_id: &str) -> Result<()> {
    sqlx::query!(
        r#"
            DELETE FROM github_connection
            WHERE user_id = ?
            "#,
        user_id
    )
    .execute(pool)
    .await?;

    debug!("Deleted GitHub connection for user '{}'", user_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::db::user;
    use crate::models::{GitHubConnection, User};

    #[tokio::test]
    async fn test_save_and_get_by_user_id() {
        let pool = get_test_pool().await;
        let test_user = User::new("user@example.com", "password123", None).unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let connection = GitHubConnection::new(
            &test_user.id,
            12345,
            "githubuser",
            Some("github@example.com".to_string()),
            "token123",
            "repo,user",
        );

        save(&pool, &connection).await.unwrap();
        let retrieved = get_by_user_id(&pool, &test_user.id).await.unwrap().unwrap();

        assert_eq!(retrieved.user_id, test_user.id);
        assert_eq!(retrieved.github_user_id, 12345);
        assert_eq!(retrieved.github_username, "githubuser");
        assert_eq!(retrieved.github_email, Some("github@example.com".to_string()));
        assert_eq!(retrieved.access_token, "token123");
        assert_eq!(retrieved.scopes, "repo,user");
    }

    #[tokio::test]
    async fn test_get_by_user_id_not_found() {
        let pool = get_test_pool().await;
        let result = get_by_user_id(&pool, "nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_save_update_existing() {
        let pool = get_test_pool().await;
        let test_user = User::new("user@example.com", "password123", None).unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let mut connection = GitHubConnection::new(
            &test_user.id,
            12345,
            "githubuser",
            Some("github@example.com".to_string()),
            "token123",
            "repo,user",
        );

        save(&pool, &connection).await.unwrap();

        connection.update_token("newtoken456", "repo,user,admin");
        save(&pool, &connection).await.unwrap();

        let retrieved = get_by_user_id(&pool, &test_user.id).await.unwrap().unwrap();
        assert_eq!(retrieved.access_token, "newtoken456");
        assert_eq!(retrieved.scopes, "repo,user,admin");
    }

    #[tokio::test]
    async fn test_delete_by_user_id() {
        let pool = get_test_pool().await;
        let test_user = User::new("user@example.com", "password123", None).unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let connection = GitHubConnection::new(
            &test_user.id,
            12345,
            "githubuser",
            None,
            "token123",
            "repo",
        );

        save(&pool, &connection).await.unwrap();
        assert!(get_by_user_id(&pool, &test_user.id).await.unwrap().is_some());

        delete_by_user_id(&pool, &test_user.id).await.unwrap();
        assert!(get_by_user_id(&pool, &test_user.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_connection_without_email() {
        let pool = get_test_pool().await;
        let test_user = User::new("user@example.com", "password123", None).unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let connection = GitHubConnection::new(
            &test_user.id,
            12345,
            "githubuser",
            None,
            "token123",
            "repo",
        );

        save(&pool, &connection).await.unwrap();
        let retrieved = get_by_user_id(&pool, &test_user.id).await.unwrap().unwrap();

        assert_eq!(retrieved.github_email, None);
    }
}
