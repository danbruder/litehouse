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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::db::{organization, user};
    use crate::models::{Organization, OrganizationMember, OrgRole, User};

    #[tokio::test]
    async fn test_save_and_get_member() {
        let pool = get_test_pool().await;
        let org = Organization::new("Test Org").unwrap();
        let test_user = User::new("test@example.com", "password123", None).unwrap();
        organization::save(&pool, &org).await.unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let member = OrganizationMember::new(&org.id, &test_user.id, OrgRole::Member);
        save(&pool, &member).await.unwrap();

        let retrieved = get_member(&pool, &org.id, &test_user.id).await.unwrap().unwrap();
        assert_eq!(retrieved.organization_id, org.id);
        assert_eq!(retrieved.user_id, test_user.id);
        assert_eq!(retrieved.role, OrgRole::Member);
    }

    #[tokio::test]
    async fn test_get_members() {
        let pool = get_test_pool().await;
        let org = Organization::new("Test Org").unwrap();
        let user1 = User::new("user1@example.com", "password123", None).unwrap();
        let user2 = User::new("user2@example.com", "password123", None).unwrap();
        organization::save(&pool, &org).await.unwrap();
        user::save(&pool, &user1).await.unwrap();
        user::save(&pool, &user2).await.unwrap();

        let member1 = OrganizationMember::new(&org.id, &user1.id, OrgRole::Owner);
        let member2 = OrganizationMember::new(&org.id, &user2.id, OrgRole::Member);
        save(&pool, &member1).await.unwrap();
        save(&pool, &member2).await.unwrap();

        let members = get_members(&pool, &org.id).await.unwrap();
        assert_eq!(members.len(), 2);
        assert!(members.iter().any(|(u, r)| u.id == user1.id && *r == OrgRole::Owner));
        assert!(members.iter().any(|(u, r)| u.id == user2.id && *r == OrgRole::Member));
    }

    #[tokio::test]
    async fn test_get_user_role() {
        let pool = get_test_pool().await;
        let org = Organization::new("Test Org").unwrap();
        let test_user = User::new("test@example.com", "password123", None).unwrap();
        organization::save(&pool, &org).await.unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let member = OrganizationMember::new(&org.id, &test_user.id, OrgRole::Admin);
        save(&pool, &member).await.unwrap();

        let role = get_user_role(&pool, &org.id, &test_user.id).await.unwrap().unwrap();
        assert_eq!(role, OrgRole::Admin);
    }

    #[tokio::test]
    async fn test_get_user_role_not_found() {
        let pool = get_test_pool().await;
        let org = Organization::new("Test Org").unwrap();
        let test_user = User::new("test@example.com", "password123", None).unwrap();
        organization::save(&pool, &org).await.unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let role = get_user_role(&pool, &org.id, &test_user.id).await.unwrap();
        assert!(role.is_none());
    }

    #[tokio::test]
    async fn test_remove_member() {
        let pool = get_test_pool().await;
        let org = Organization::new("Test Org").unwrap();
        let test_user = User::new("test@example.com", "password123", None).unwrap();
        organization::save(&pool, &org).await.unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let member = OrganizationMember::new(&org.id, &test_user.id, OrgRole::Member);
        save(&pool, &member).await.unwrap();
        assert!(get_member(&pool, &org.id, &test_user.id).await.unwrap().is_some());

        remove_member(&pool, &org.id, &test_user.id).await.unwrap();
        assert!(get_member(&pool, &org.id, &test_user.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_update_role() {
        let pool = get_test_pool().await;
        let org = Organization::new("Test Org").unwrap();
        let test_user = User::new("test@example.com", "password123", None).unwrap();
        organization::save(&pool, &org).await.unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let member = OrganizationMember::new(&org.id, &test_user.id, OrgRole::Member);
        save(&pool, &member).await.unwrap();

        update_role(&pool, &org.id, &test_user.id, OrgRole::Admin).await.unwrap();
        let retrieved = get_member(&pool, &org.id, &test_user.id).await.unwrap().unwrap();
        assert_eq!(retrieved.role, OrgRole::Admin);
    }

    #[tokio::test]
    async fn test_count_owners() {
        let pool = get_test_pool().await;
        let org = Organization::new("Test Org").unwrap();
        let user1 = User::new("user1@example.com", "password123", None).unwrap();
        let user2 = User::new("user2@example.com", "password123", None).unwrap();
        let user3 = User::new("user3@example.com", "password123", None).unwrap();
        organization::save(&pool, &org).await.unwrap();
        user::save(&pool, &user1).await.unwrap();
        user::save(&pool, &user2).await.unwrap();
        user::save(&pool, &user3).await.unwrap();

        let member1 = OrganizationMember::new(&org.id, &user1.id, OrgRole::Owner);
        let member2 = OrganizationMember::new(&org.id, &user2.id, OrgRole::Owner);
        let member3 = OrganizationMember::new(&org.id, &user3.id, OrgRole::Member);
        save(&pool, &member1).await.unwrap();
        save(&pool, &member2).await.unwrap();
        save(&pool, &member3).await.unwrap();

        let count = count_owners(&pool, &org.id).await.unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_get_user_org_ids() {
        let pool = get_test_pool().await;
        let org1 = Organization::new("Org 1").unwrap();
        let org2 = Organization::new("Org 2").unwrap();
        let test_user = User::new("test@example.com", "password123", None).unwrap();
        organization::save(&pool, &org1).await.unwrap();
        organization::save(&pool, &org2).await.unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let member1 = OrganizationMember::new(&org1.id, &test_user.id, OrgRole::Member);
        let member2 = OrganizationMember::new(&org2.id, &test_user.id, OrgRole::Member);
        save(&pool, &member1).await.unwrap();
        save(&pool, &member2).await.unwrap();

        let org_ids = get_user_org_ids(&pool, &test_user.id).await.unwrap();
        assert_eq!(org_ids.len(), 2);
        assert!(org_ids.contains(&org1.id));
        assert!(org_ids.contains(&org2.id));
    }

    #[tokio::test]
    async fn test_save_update_existing() {
        let pool = get_test_pool().await;
        let org = Organization::new("Test Org").unwrap();
        let test_user = User::new("test@example.com", "password123", None).unwrap();
        organization::save(&pool, &org).await.unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let member1 = OrganizationMember::new(&org.id, &test_user.id, OrgRole::Member);
        save(&pool, &member1).await.unwrap();

        let member2 = OrganizationMember::new(&org.id, &test_user.id, OrgRole::Admin);
        save(&pool, &member2).await.unwrap();

        let retrieved = get_member(&pool, &org.id, &test_user.id).await.unwrap().unwrap();
        assert_eq!(retrieved.role, OrgRole::Admin);
    }
}
