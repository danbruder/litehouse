use super::*;
use crate::models::{OrganizationMember, OrgRole, User};

/// Save an organization member to the database
#[instrument(skip(pool, member))]
pub async fn save(pool: &Pool<Sqlite>, member: &OrganizationMember) -> Result<()> {
    let role_str = member.role.to_string();
    sqlx::query!(
        r#"
            INSERT INTO organization_member (
                id, organization_id, user_id, role, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(organization_id, user_id) DO UPDATE SET
                role = excluded.role,
                updated_at = excluded.updated_at
            "#,
        member.id,
        member.organization_id,
        member.user_id,
        role_str,
        member.created_at,
        member.updated_at
    )
    .execute(pool)
    .await?;

    debug!(
        "Saved organization member (org: {}, user: {}, role: {})",
        member.organization_id, member.user_id, member.role
    );
    Ok(())
}

/// Get all members of an organization with user details
#[instrument(skip(pool))]
pub async fn get_members(pool: &Pool<Sqlite>, org_id: &str) -> Result<Vec<(User, OrgRole)>> {
    let rows = sqlx::query!(
        r#"
            SELECT u.id, u.email, u.password_hash, u.full_name, u.is_active,
                   u.email_verified, u.created_at as user_created_at,
                   u.updated_at as user_updated_at, om.role
            FROM user u
            INNER JOIN organization_member om ON u.id = om.user_id
            WHERE om.organization_id = ?
            ORDER BY om.role, u.email
            "#,
        org_id
    )
    .fetch_all(pool)
    .await?;

    let members = rows
        .into_iter()
        .map(|row| {
            let user = User {
                id: row.id,
                email: row.email,
                password_hash: row.password_hash,
                full_name: row.full_name,
                is_active: row.is_active,
                email_verified: row.email_verified,
                created_at: row.user_created_at.into(),
                updated_at: row.user_updated_at.into(),
            };
            let role: OrgRole = row.role.parse().unwrap_or(OrgRole::Member);
            (user, role)
        })
        .collect();

    Ok(members)
}

/// Get a user's role in an organization
#[instrument(skip(pool))]
pub async fn get_user_role(
    pool: &Pool<Sqlite>,
    org_id: &str,
    user_id: &str,
) -> Result<Option<OrgRole>> {
    let row = sqlx::query!(
        r#"
            SELECT role
            FROM organization_member
            WHERE organization_id = ? AND user_id = ?
            "#,
        org_id,
        user_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.and_then(|r| r.role.parse().ok()))
}

/// Get organization member record
#[instrument(skip(pool))]
pub async fn get_member(
    pool: &Pool<Sqlite>,
    org_id: &str,
    user_id: &str,
) -> Result<Option<OrganizationMember>> {
    let row = sqlx::query!(
        r#"
            SELECT id, organization_id, user_id, role, created_at, updated_at
            FROM organization_member
            WHERE organization_id = ? AND user_id = ?
            "#,
        org_id,
        user_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| OrganizationMember {
        id: r.id,
        organization_id: r.organization_id,
        user_id: r.user_id,
        role: r.role.parse().unwrap_or(OrgRole::Member),
        created_at: r.created_at.into(),
        updated_at: r.updated_at.into(),
    }))
}

/// Remove a member from an organization
#[instrument(skip(pool))]
pub async fn remove_member(pool: &Pool<Sqlite>, org_id: &str, user_id: &str) -> Result<()> {
    sqlx::query!(
        r#"
            DELETE FROM organization_member
            WHERE organization_id = ? AND user_id = ?
            "#,
        org_id,
        user_id
    )
    .execute(pool)
    .await?;

    debug!("Removed user '{}' from organization '{}'", user_id, org_id);
    Ok(())
}

/// Update a member's role in an organization
#[instrument(skip(pool))]
pub async fn update_role(
    pool: &Pool<Sqlite>,
    org_id: &str,
    user_id: &str,
    role: OrgRole,
) -> Result<()> {
    let role_str = role.to_string();
    sqlx::query!(
        r#"
            UPDATE organization_member
            SET role = ?, updated_at = datetime('now')
            WHERE organization_id = ? AND user_id = ?
            "#,
        role_str,
        org_id,
        user_id
    )
    .execute(pool)
    .await?;

    debug!(
        "Updated role for user '{}' in organization '{}' to '{}'",
        user_id, org_id, role
    );
    Ok(())
}

/// Count owners in an organization
#[instrument(skip(pool))]
pub async fn count_owners(pool: &Pool<Sqlite>, org_id: &str) -> Result<i64> {
    let result = sqlx::query!(
        r#"
            SELECT COUNT(*) as count
            FROM organization_member
            WHERE organization_id = ? AND role = 'owner'
            "#,
        org_id
    )
    .fetch_one(pool)
    .await?;

    Ok(result.count as i64)
}

/// Get all organizations for a user
#[instrument(skip(pool))]
pub async fn get_user_org_ids(pool: &Pool<Sqlite>, user_id: &str) -> Result<Vec<String>> {
    let rows = sqlx::query!(
        r#"
            SELECT organization_id
            FROM organization_member
            WHERE user_id = ?
            "#,
        user_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|r| r.organization_id).collect())
}
