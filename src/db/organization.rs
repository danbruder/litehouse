use super::*;
use crate::models::{Organization, OrganizationWithRole, OrgRole};

/// Save an organization to the database
#[instrument(skip(pool, org))]
pub async fn save(pool: &Pool<Sqlite>, org: &Organization) -> Result<()> {
    sqlx::query!(
        r#"
            INSERT INTO organization (
                id, name, slug, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                slug = excluded.slug,
                updated_at = excluded.updated_at
            "#,
        org.id,
        org.name,
        org.slug,
        org.created_at,
        org.updated_at
    )
    .execute(pool)
    .await?;

    debug!("Saved organization '{}'", org.name);
    Ok(())
}

/// Get an organization by ID
#[instrument(skip(pool))]
pub async fn get_by_id(pool: &Pool<Sqlite>, id: &str) -> Result<Option<Organization>> {
    let org = sqlx::query_as!(
        Organization,
        r#"
            SELECT id, name, slug, created_at, updated_at
            FROM organization
            WHERE id = ?
            "#,
        id
    )
    .fetch_optional(pool)
    .await?;

    Ok(org)
}

/// Get an organization by slug
#[instrument(skip(pool))]
pub async fn get_by_slug(pool: &Pool<Sqlite>, slug: &str) -> Result<Option<Organization>> {
    let org = sqlx::query_as!(
        Organization,
        r#"
            SELECT id, name, slug, created_at, updated_at
            FROM organization
            WHERE slug = ?
            "#,
        slug
    )
    .fetch_optional(pool)
    .await?;

    Ok(org)
}

/// Get an organization by name
#[instrument(skip(pool))]
pub async fn get_by_name(pool: &Pool<Sqlite>, name: &str) -> Result<Option<Organization>> {
    let org = sqlx::query_as!(
        Organization,
        r#"
            SELECT id, name, slug, created_at, updated_at
            FROM organization
            WHERE name = ?
            "#,
        name
    )
    .fetch_optional(pool)
    .await?;

    Ok(org)
}

/// Get all organizations for a user with their roles
#[instrument(skip(pool))]
pub async fn get_user_organizations(pool: &Pool<Sqlite>, user_id: &str) -> Result<Vec<OrganizationWithRole>> {
    let rows = sqlx::query!(
        r#"
            SELECT o.id, o.name, o.slug, o.created_at, o.updated_at, om.role
            FROM organization o
            INNER JOIN organization_member om ON o.id = om.organization_id
            WHERE om.user_id = ?
            ORDER BY o.name
            "#,
        user_id
    )
    .fetch_all(pool)
    .await?;

    let orgs = rows
        .into_iter()
        .map(|row| {
            let org = Organization {
                id: row.id,
                name: row.name,
                slug: row.slug,
                created_at: row.created_at.into(),
                updated_at: row.updated_at.into(),
            };
            let role: OrgRole = row.role.parse().unwrap_or(OrgRole::Member);
            OrganizationWithRole {
                organization: org,
                role,
            }
        })
        .collect();

    Ok(orgs)
}

/// Get all organizations
#[instrument(skip(pool))]
pub async fn get_all(pool: &Pool<Sqlite>) -> Result<Vec<Organization>> {
    let orgs = sqlx::query_as!(
        Organization,
        r#"
            SELECT id, name, slug, created_at, updated_at
            FROM organization
            ORDER BY name
            "#
    )
    .fetch_all(pool)
    .await?;

    Ok(orgs)
}

/// Delete an organization
#[instrument(skip(pool))]
pub async fn delete(pool: &Pool<Sqlite>, org_id: &str) -> Result<()> {
    sqlx::query!(
        r#"
            DELETE FROM organization
            WHERE id = ?
            "#,
        org_id
    )
    .execute(pool)
    .await?;

    debug!("Deleted organization '{}'", org_id);
    Ok(())
}

/// Check if a user has access to an organization
#[instrument(skip(pool))]
pub async fn user_has_access(pool: &Pool<Sqlite>, org_id: &str, user_id: &str) -> Result<bool> {
    let result = sqlx::query!(
        r#"
            SELECT COUNT(*) as count
            FROM organization_member
            WHERE organization_id = ? AND user_id = ?
            "#,
        org_id,
        user_id
    )
    .fetch_one(pool)
    .await?;

    Ok(result.count > 0)
}
