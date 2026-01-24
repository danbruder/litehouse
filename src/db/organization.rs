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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::db::{organization_member, user};
    use crate::models::{Organization, OrganizationMember, OrgRole, User};

    #[tokio::test]
    async fn test_save_and_get_by_id() {
        let pool = get_test_pool().await;
        let org = Organization::new("Test Org").unwrap();

        save(&pool, &org).await.unwrap();
        let retrieved = get_by_id(&pool, &org.id).await.unwrap().unwrap();

        assert_eq!(retrieved.id, org.id);
        assert_eq!(retrieved.name, "Test Org");
        assert_eq!(retrieved.slug, "test-org");
    }

    #[tokio::test]
    async fn test_get_by_slug() {
        let pool = get_test_pool().await;
        let org = Organization::new("My Company").unwrap();

        save(&pool, &org).await.unwrap();
        let retrieved = get_by_slug(&pool, "my-company").await.unwrap().unwrap();

        assert_eq!(retrieved.id, org.id);
        assert_eq!(retrieved.name, "My Company");
    }

    #[tokio::test]
    async fn test_get_by_name() {
        let pool = get_test_pool().await;
        let org = Organization::new("Test Org").unwrap();

        save(&pool, &org).await.unwrap();
        let retrieved = get_by_name(&pool, "Test Org").await.unwrap().unwrap();

        assert_eq!(retrieved.id, org.id);
    }

    #[tokio::test]
    async fn test_get_by_id_not_found() {
        let pool = get_test_pool().await;
        let result = get_by_id(&pool, "nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_get_user_organizations() {
        let pool = get_test_pool().await;
        let org1 = Organization::new("Org 1").unwrap();
        let org2 = Organization::new("Org 2").unwrap();
        let test_user = User::new("test@example.com", "password123", None).unwrap();

        save(&pool, &org1).await.unwrap();
        save(&pool, &org2).await.unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let member1 = OrganizationMember::new(&org1.id, &test_user.id, OrgRole::Owner);
        let member2 = OrganizationMember::new(&org2.id, &test_user.id, OrgRole::Member);
        organization_member::save(&pool, &member1).await.unwrap();
        organization_member::save(&pool, &member2).await.unwrap();

        let orgs = get_user_organizations(&pool, &test_user.id).await.unwrap();
        assert_eq!(orgs.len(), 2);
        assert!(orgs.iter().any(|o| o.organization.id == org1.id && o.role == OrgRole::Owner));
        assert!(orgs.iter().any(|o| o.organization.id == org2.id && o.role == OrgRole::Member));
    }

    #[tokio::test]
    async fn test_get_all() {
        let pool = get_test_pool().await;
        let org1 = Organization::new("Org A").unwrap();
        let org2 = Organization::new("Org B").unwrap();
        let org3 = Organization::new("Org C").unwrap();

        save(&pool, &org1).await.unwrap();
        save(&pool, &org2).await.unwrap();
        save(&pool, &org3).await.unwrap();

        let all_orgs = get_all(&pool).await.unwrap();
        // Should be ordered by name, verify all orgs are present
        let org_names: Vec<String> = all_orgs.iter().map(|o| o.name.clone()).collect();
        assert!(org_names.contains(&"Org A".to_string()));
        assert!(org_names.contains(&"Org B".to_string()));
        assert!(org_names.contains(&"Org C".to_string()));
        // Verify ordering (alphabetical)
        let org_a_idx = org_names.iter().position(|n| n == "Org A").unwrap();
        let org_b_idx = org_names.iter().position(|n| n == "Org B").unwrap();
        let org_c_idx = org_names.iter().position(|n| n == "Org C").unwrap();
        assert!(org_a_idx < org_b_idx);
        assert!(org_b_idx < org_c_idx);
    }

    #[tokio::test]
    async fn test_delete() {
        let pool = get_test_pool().await;
        let org = Organization::new("To Delete").unwrap();

        save(&pool, &org).await.unwrap();
        assert!(get_by_id(&pool, &org.id).await.unwrap().is_some());

        delete(&pool, &org.id).await.unwrap();
        assert!(get_by_id(&pool, &org.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_user_has_access() {
        let pool = get_test_pool().await;
        let org = Organization::new("Test Org").unwrap();
        let test_user = User::new("test@example.com", "password123", None).unwrap();

        save(&pool, &org).await.unwrap();
        user::save(&pool, &test_user).await.unwrap();

        assert!(!user_has_access(&pool, &org.id, &test_user.id).await.unwrap());

        let member = OrganizationMember::new(&org.id, &test_user.id, OrgRole::Member);
        organization_member::save(&pool, &member).await.unwrap();

        assert!(user_has_access(&pool, &org.id, &test_user.id).await.unwrap());
    }

    #[tokio::test]
    async fn test_save_update_existing() {
        let pool = get_test_pool().await;
        let mut org = Organization::new("Original Name").unwrap();
        let original_id = org.id.clone();

        save(&pool, &org).await.unwrap();
        org.update_name("Updated Name").unwrap();
        save(&pool, &org).await.unwrap();

        let retrieved = get_by_id(&pool, &original_id).await.unwrap().unwrap();
        assert_eq!(retrieved.name, "Updated Name");
        assert_eq!(retrieved.slug, "updated-name");
    }
}
