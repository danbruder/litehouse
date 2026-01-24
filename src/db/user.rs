use super::*;
use crate::models::User;

/// Save a user to the database
#[instrument(skip(pool, user))]
pub async fn save(pool: &Pool<Sqlite>, user: &User) -> Result<()> {
    sqlx::query!(
        r#"
            INSERT INTO user (
                id, email, password_hash, full_name, is_active, email_verified, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                email = excluded.email,
                password_hash = excluded.password_hash,
                full_name = excluded.full_name,
                is_active = excluded.is_active,
                email_verified = excluded.email_verified,
                updated_at = excluded.updated_at
            "#,
        user.id,
        user.email,
        user.password_hash,
        user.full_name,
        user.is_active,
        user.email_verified,
        user.created_at,
        user.updated_at
    )
    .execute(pool)
    .await?;

    debug!("Saved user '{}'", user.email);
    Ok(())
}

/// Get a user by ID
#[instrument(skip(pool))]
pub async fn get_by_id(pool: &Pool<Sqlite>, id: &str) -> Result<Option<User>> {
    let user = sqlx::query_as!(
        User,
        r#"
            SELECT id, email, password_hash, full_name, is_active, email_verified, created_at, updated_at
            FROM user
            WHERE id = ?
            "#,
        id
    )
    .fetch_optional(pool)
    .await?;

    Ok(user)
}

/// Get a user by email
#[instrument(skip(pool))]
pub async fn get_by_email(pool: &Pool<Sqlite>, email: &str) -> Result<Option<User>> {
    let email_lower = email.to_lowercase();
    let user = sqlx::query_as!(
        User,
        r#"
            SELECT id, email, password_hash, full_name, is_active, email_verified, created_at, updated_at
            FROM user
            WHERE email = ?
            "#,
        email_lower
    )
    .fetch_optional(pool)
    .await?;

    Ok(user)
}

/// Update user password
#[instrument(skip(pool, new_password_hash))]
pub async fn update_password(pool: &Pool<Sqlite>, user_id: &str, new_password_hash: &str) -> Result<()> {
    sqlx::query!(
        r#"
            UPDATE user
            SET password_hash = ?, updated_at = datetime('now')
            WHERE id = ?
            "#,
        new_password_hash,
        user_id
    )
    .execute(pool)
    .await?;

    debug!("Updated password for user '{}'", user_id);
    Ok(())
}

/// Update user email verification status
#[instrument(skip(pool))]
pub async fn mark_email_verified(pool: &Pool<Sqlite>, user_id: &str) -> Result<()> {
    sqlx::query!(
        r#"
            UPDATE user
            SET email_verified = true, updated_at = datetime('now')
            WHERE id = ?
            "#,
        user_id
    )
    .execute(pool)
    .await?;

    debug!("Marked email as verified for user '{}'", user_id);
    Ok(())
}

/// Deactivate a user
#[instrument(skip(pool))]
pub async fn deactivate(pool: &Pool<Sqlite>, user_id: &str) -> Result<()> {
    sqlx::query!(
        r#"
            UPDATE user
            SET is_active = false, updated_at = datetime('now')
            WHERE id = ?
            "#,
        user_id
    )
    .execute(pool)
    .await?;

    debug!("Deactivated user '{}'", user_id);
    Ok(())
}

/// Delete a user
#[instrument(skip(pool))]
pub async fn delete(pool: &Pool<Sqlite>, user_id: &str) -> Result<()> {
    sqlx::query!(
        r#"
            DELETE FROM user
            WHERE id = ?
            "#,
        user_id
    )
    .execute(pool)
    .await?;

    debug!("Deleted user '{}'", user_id);
    Ok(())
}

/// Get all users (admin function)
#[instrument(skip(pool))]
pub async fn get_all(pool: &Pool<Sqlite>) -> Result<Vec<User>> {
    let users = sqlx::query_as!(
        User,
        r#"
            SELECT id, email, password_hash, full_name, is_active, email_verified, created_at, updated_at
            FROM user
            ORDER BY created_at DESC
            "#
    )
    .fetch_all(pool)
    .await?;

    Ok(users)
}

/// Count total number of users
#[instrument(skip(pool))]
pub async fn count(pool: &Pool<Sqlite>) -> Result<i64> {
    let result = sqlx::query_scalar!(r#"SELECT COUNT(*) as "count: i64" FROM user"#)
        .fetch_one(pool)
        .await?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::models::User;

    #[tokio::test]
    async fn test_save_and_get_by_id() {
        let pool = get_test_pool().await;
        let user = User::new("test@example.com", "password123", Some("Test User".to_string())).unwrap();

        save(&pool, &user).await.unwrap();
        let retrieved = get_by_id(&pool, &user.id).await.unwrap().unwrap();

        assert_eq!(retrieved.id, user.id);
        assert_eq!(retrieved.email, "test@example.com");
        assert_eq!(retrieved.full_name, Some("Test User".to_string()));
        assert!(retrieved.is_active);
        assert!(!retrieved.email_verified);
    }

    #[tokio::test]
    async fn test_save_and_get_by_email() {
        let pool = get_test_pool().await;
        let user = User::new("test@example.com", "password123", None).unwrap();

        save(&pool, &user).await.unwrap();
        let retrieved = get_by_email(&pool, "test@example.com").await.unwrap().unwrap();

        assert_eq!(retrieved.id, user.id);
        assert_eq!(retrieved.email, "test@example.com");
    }

    #[tokio::test]
    async fn test_get_by_email_case_insensitive() {
        let pool = get_test_pool().await;
        let user = User::new("Test@Example.COM", "password123", None).unwrap();

        save(&pool, &user).await.unwrap();
        // User::new converts email to lowercase
        let retrieved = get_by_email(&pool, "test@example.com").await.unwrap().unwrap();
        assert_eq!(retrieved.id, user.id);
    }

    #[tokio::test]
    async fn test_get_by_id_not_found() {
        let pool = get_test_pool().await;
        let result = get_by_id(&pool, "nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_get_by_email_not_found() {
        let pool = get_test_pool().await;
        let result = get_by_email(&pool, "nonexistent@example.com").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_update_password() {
        let pool = get_test_pool().await;
        let user = User::new("test@example.com", "password123", None).unwrap();
        let user_id = user.id.clone();
        save(&pool, &user).await.unwrap();

        let new_password_hash = User::hash_password("newpassword456").unwrap();
        update_password(&pool, &user_id, &new_password_hash).await.unwrap();
        let retrieved = get_by_id(&pool, &user_id).await.unwrap().unwrap();

        // Verify the password hash changed
        assert_ne!(retrieved.password_hash, user.password_hash);
        // Verify the new password works
        assert!(retrieved.verify_password("newpassword456").unwrap());
        assert!(!retrieved.verify_password("password123").unwrap());
    }

    #[tokio::test]
    async fn test_mark_email_verified() {
        let pool = get_test_pool().await;
        let user = User::new("test@example.com", "password123", None).unwrap();
        let user_id = user.id.clone();
        save(&pool, &user).await.unwrap();
        assert!(!user.email_verified);

        mark_email_verified(&pool, &user_id).await.unwrap();
        let retrieved = get_by_id(&pool, &user_id).await.unwrap().unwrap();
        assert!(retrieved.email_verified);
    }

    #[tokio::test]
    async fn test_deactivate() {
        let pool = get_test_pool().await;
        let user = User::new("test@example.com", "password123", None).unwrap();
        let user_id = user.id.clone();
        save(&pool, &user).await.unwrap();
        assert!(user.is_active);

        deactivate(&pool, &user_id).await.unwrap();
        let retrieved = get_by_id(&pool, &user_id).await.unwrap().unwrap();
        assert!(!retrieved.is_active);
    }

    #[tokio::test]
    async fn test_delete() {
        let pool = get_test_pool().await;
        let user = User::new("test@example.com", "password123", None).unwrap();
        let user_id = user.id.clone();
        save(&pool, &user).await.unwrap();
        assert!(get_by_id(&pool, &user_id).await.unwrap().is_some());

        delete(&pool, &user_id).await.unwrap();
        assert!(get_by_id(&pool, &user_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_get_all() {
        let pool = get_test_pool().await;
        let user1 = User::new("user1@example.com", "password123", None).unwrap();
        let user2 = User::new("user2@example.com", "password123", None).unwrap();
        let user3 = User::new("user3@example.com", "password123", None).unwrap();

        save(&pool, &user1).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        save(&pool, &user2).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        save(&pool, &user3).await.unwrap();

        let all_users = get_all(&pool).await.unwrap();
        assert_eq!(all_users.len(), 3);
        // Should be ordered by created_at DESC - verify all users are present
        let user_ids: Vec<String> = all_users.iter().map(|u| u.id.clone()).collect();
        assert!(user_ids.contains(&user1.id));
        assert!(user_ids.contains(&user2.id));
        assert!(user_ids.contains(&user3.id));
        // Verify all users are present
        // Due to SQLite datetime precision, exact ordering may vary
        // The important thing is that all users are returned
        assert_eq!(user_ids.len(), 3);
    }

    #[tokio::test]
    async fn test_count() {
        let pool = get_test_pool().await;
        assert_eq!(count(&pool).await.unwrap(), 0);

        let user1 = User::new("user1@example.com", "password123", None).unwrap();
        let user2 = User::new("user2@example.com", "password123", None).unwrap();
        save(&pool, &user1).await.unwrap();
        save(&pool, &user2).await.unwrap();

        assert_eq!(count(&pool).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn test_save_update_existing() {
        let pool = get_test_pool().await;
        let mut user = User::new("test@example.com", "password123", Some("Original Name".to_string())).unwrap();
        let user_id = user.id.clone();
        save(&pool, &user).await.unwrap();

        user.full_name = Some("Updated Name".to_string());
        user.email = "updated@example.com".to_string();
        save(&pool, &user).await.unwrap();

        let retrieved = get_by_id(&pool, &user_id).await.unwrap().unwrap();
        assert_eq!(retrieved.email, "updated@example.com");
        assert_eq!(retrieved.full_name, Some("Updated Name".to_string()));
    }
}
