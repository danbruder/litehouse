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
