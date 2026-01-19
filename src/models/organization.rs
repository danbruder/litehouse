use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::models::{now, UtcDateTime};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Organization {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub created_at: UtcDateTime,
    pub updated_at: UtcDateTime,
}

#[derive(Debug, thiserror::Error)]
pub enum OrganizationError {
    #[error("Invalid organization name: {0}")]
    InvalidName(String),
    #[error("Invalid slug: {0}")]
    InvalidSlug(String),
}

impl Organization {
    /// Create a new organization
    pub fn new(name: &str) -> Result<Self, OrganizationError> {
        if name.is_empty() || name.len() > 100 {
            return Err(OrganizationError::InvalidName(name.to_string()));
        }

        let slug = Self::generate_slug(name);
        let now = now();

        Ok(Self {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            slug,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    /// Generate a URL-friendly slug from the organization name
    fn generate_slug(name: &str) -> String {
        name.to_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c
                } else if c.is_whitespace() || c == '-' || c == '_' {
                    '-'
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    }

    /// Update organization name and regenerate slug
    pub fn update_name(&mut self, new_name: &str) -> Result<(), OrganizationError> {
        if new_name.is_empty() || new_name.len() > 100 {
            return Err(OrganizationError::InvalidName(new_name.to_string()));
        }

        self.name = new_name.to_string();
        self.slug = Self::generate_slug(new_name);
        self.updated_at = now();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_organization() {
        let org = Organization::new("My Company");
        assert!(org.is_ok());
        let org = org.unwrap();
        assert_eq!(org.name, "My Company");
        assert_eq!(org.slug, "my-company");
    }

    #[test]
    fn test_generate_slug() {
        assert_eq!(Organization::generate_slug("My Company"), "my-company");
        assert_eq!(Organization::generate_slug("ACME Corp!"), "acme-corp_");
        assert_eq!(
            Organization::generate_slug("Test  Multiple   Spaces"),
            "test-multiple-spaces"
        );
        assert_eq!(
            Organization::generate_slug("Special@#$Characters"),
            "special___characters"
        );
    }

    #[test]
    fn test_invalid_name_empty() {
        let result = Organization::new("");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OrganizationError::InvalidName(_)
        ));
    }

    #[test]
    fn test_invalid_name_too_long() {
        let long_name = "a".repeat(101);
        let result = Organization::new(&long_name);
        assert!(result.is_err());
    }

    #[test]
    fn test_update_name() {
        let mut org = Organization::new("Original Name").unwrap();
        let old_slug = org.slug.clone();

        org.update_name("New Name").unwrap();
        assert_eq!(org.name, "New Name");
        assert_eq!(org.slug, "new-name");
        assert_ne!(org.slug, old_slug);
    }
}
