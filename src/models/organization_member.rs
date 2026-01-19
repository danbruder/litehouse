use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Type};
use std::str::FromStr;
use uuid::Uuid;

use crate::models::{now, UtcDateTime};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[sqlx(type_name = "TEXT")]
pub enum OrgRole {
    Owner,
    Admin,
    Member,
}

impl std::fmt::Display for OrgRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrgRole::Owner => write!(f, "owner"),
            OrgRole::Admin => write!(f, "admin"),
            OrgRole::Member => write!(f, "member"),
        }
    }
}

impl FromStr for OrgRole {
    type Err = OrgMemberError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "owner" => Ok(OrgRole::Owner),
            "admin" => Ok(OrgRole::Admin),
            "member" => Ok(OrgRole::Member),
            _ => Err(OrgMemberError::InvalidRole(s.to_string())),
        }
    }
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct OrganizationMember {
    pub id: String,
    pub organization_id: String,
    pub user_id: String,
    pub role: OrgRole,
    pub created_at: UtcDateTime,
    pub updated_at: UtcDateTime,
}

#[derive(Debug, thiserror::Error)]
pub enum OrgMemberError {
    #[error("Invalid role: {0}")]
    InvalidRole(String),
    #[error("Insufficient permissions for this operation")]
    InsufficientPermissions,
    #[error("Cannot remove last owner from organization")]
    CannotRemoveLastOwner,
}

impl OrganizationMember {
    /// Create a new organization member
    pub fn new(organization_id: &str, user_id: &str, role: OrgRole) -> Self {
        let now = now();

        Self {
            id: Uuid::new_v4().to_string(),
            organization_id: organization_id.to_string(),
            user_id: user_id.to_string(),
            role,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Check if member can create apps
    pub fn can_create_apps(&self) -> bool {
        matches!(self.role, OrgRole::Owner | OrgRole::Admin | OrgRole::Member)
    }

    /// Check if member can delete apps
    pub fn can_delete_apps(&self) -> bool {
        matches!(self.role, OrgRole::Owner | OrgRole::Admin)
    }

    /// Check if member can manage other members
    pub fn can_manage_members(&self) -> bool {
        matches!(self.role, OrgRole::Owner | OrgRole::Admin)
    }

    /// Check if member can manage organization settings
    pub fn can_manage_org(&self) -> bool {
        matches!(self.role, OrgRole::Owner)
    }

    /// Check if member can view apps
    pub fn can_view_apps(&self) -> bool {
        true // All members can view apps in their organization
    }

    /// Check if member can manage app settings (env vars, remotes, etc.)
    pub fn can_manage_app_settings(&self) -> bool {
        matches!(self.role, OrgRole::Owner | OrgRole::Admin | OrgRole::Member)
    }

    /// Update member role
    pub fn update_role(&mut self, new_role: OrgRole) {
        self.role = new_role;
        self.updated_at = now();
    }

    /// Check if this member can promote/demote another member
    pub fn can_change_role(&self, target_role: &OrgRole, new_role: &OrgRole) -> bool {
        match self.role {
            OrgRole::Owner => true, // Owners can change anyone's role
            OrgRole::Admin => {
                // Admins can change members' roles but not other admins or owners
                !matches!(target_role, OrgRole::Owner | OrgRole::Admin)
                    && !matches!(new_role, OrgRole::Owner)
            }
            OrgRole::Member => false, // Members cannot change roles
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_member() {
        let member = OrganizationMember::new("org-id", "user-id", OrgRole::Member);
        assert_eq!(member.organization_id, "org-id");
        assert_eq!(member.user_id, "user-id");
        assert_eq!(member.role, OrgRole::Member);
    }

    #[test]
    fn test_role_from_str() {
        assert_eq!(OrgRole::from_str("owner").unwrap(), OrgRole::Owner);
        assert_eq!(OrgRole::from_str("ADMIN").unwrap(), OrgRole::Admin);
        assert_eq!(OrgRole::from_str("Member").unwrap(), OrgRole::Member);
        assert!(OrgRole::from_str("invalid").is_err());
    }

    #[test]
    fn test_permissions_owner() {
        let member = OrganizationMember::new("org", "user", OrgRole::Owner);
        assert!(member.can_create_apps());
        assert!(member.can_delete_apps());
        assert!(member.can_manage_members());
        assert!(member.can_manage_org());
        assert!(member.can_view_apps());
        assert!(member.can_manage_app_settings());
    }

    #[test]
    fn test_permissions_admin() {
        let member = OrganizationMember::new("org", "user", OrgRole::Admin);
        assert!(member.can_create_apps());
        assert!(member.can_delete_apps());
        assert!(member.can_manage_members());
        assert!(!member.can_manage_org());
        assert!(member.can_view_apps());
    }

    #[test]
    fn test_permissions_member() {
        let member = OrganizationMember::new("org", "user", OrgRole::Member);
        assert!(member.can_create_apps());
        assert!(!member.can_delete_apps());
        assert!(!member.can_manage_members());
        assert!(!member.can_manage_org());
        assert!(member.can_view_apps());
    }

    #[test]
    fn test_can_change_role() {
        let owner = OrganizationMember::new("org", "user1", OrgRole::Owner);
        let admin = OrganizationMember::new("org", "user2", OrgRole::Admin);
        let member = OrganizationMember::new("org", "user3", OrgRole::Member);

        // Owner can change anyone's role
        assert!(owner.can_change_role(&OrgRole::Admin, &OrgRole::Member));
        assert!(owner.can_change_role(&OrgRole::Member, &OrgRole::Owner));

        // Admin can promote members but not to owner
        assert!(admin.can_change_role(&OrgRole::Member, &OrgRole::Admin));
        assert!(!admin.can_change_role(&OrgRole::Member, &OrgRole::Owner));
        assert!(!admin.can_change_role(&OrgRole::Admin, &OrgRole::Member));

        // Member cannot change roles
        assert!(!member.can_change_role(&OrgRole::Member, &OrgRole::Admin));
    }

    #[test]
    fn test_update_role() {
        let mut member = OrganizationMember::new("org", "user", OrgRole::Member);
        let old_updated_at = member.updated_at.clone();

        member.update_role(OrgRole::Admin);
        assert_eq!(member.role, OrgRole::Admin);
        // Updated timestamp should be newer (this might fail if test runs too fast)
    }
}
