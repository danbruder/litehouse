-- migrations/20260118_auth_system.sql
-- Auth and organization system

-- USERS TABLE
CREATE TABLE IF NOT EXISTS user (
    id TEXT PRIMARY KEY NOT NULL,
    email TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    full_name TEXT NULL,
    is_active BOOLEAN NOT NULL DEFAULT 1,
    email_verified BOOLEAN NOT NULL DEFAULT 0,

    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_user_email ON user(email);
CREATE INDEX IF NOT EXISTS idx_user_active ON user(is_active);

-- ORGANIZATIONS TABLE
CREATE TABLE IF NOT EXISTS organization (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT UNIQUE NOT NULL,
    slug TEXT UNIQUE NOT NULL,

    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_org_slug ON organization(slug);
CREATE INDEX IF NOT EXISTS idx_org_name ON organization(name);

-- ORGANIZATION MEMBERSHIPS TABLE
CREATE TABLE IF NOT EXISTS organization_member (
    id TEXT PRIMARY KEY NOT NULL,
    organization_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    role TEXT NOT NULL,

    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL,

    FOREIGN KEY (organization_id) REFERENCES organization(id) ON DELETE CASCADE,
    FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE,
    UNIQUE(organization_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_org_member_org ON organization_member(organization_id);
CREATE INDEX IF NOT EXISTS idx_org_member_user ON organization_member(user_id);
CREATE INDEX IF NOT EXISTS idx_org_member_role ON organization_member(role);

-- REFRESH TOKENS TABLE (for token revocation)
CREATE TABLE IF NOT EXISTS refresh_token (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    token_hash TEXT UNIQUE NOT NULL,
    expires_at DATETIME NOT NULL,
    revoked BOOLEAN NOT NULL DEFAULT 0,

    created_at DATETIME NOT NULL,

    FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_refresh_token_user ON refresh_token(user_id);
CREATE INDEX IF NOT EXISTS idx_refresh_token_hash ON refresh_token(token_hash);
CREATE INDEX IF NOT EXISTS idx_refresh_token_expires ON refresh_token(expires_at);
CREATE INDEX IF NOT EXISTS idx_refresh_token_revoked ON refresh_token(revoked);

-- ADD ORGANIZATION_ID TO APP TABLE
ALTER TABLE app ADD COLUMN organization_id TEXT NULL REFERENCES organization(id) ON DELETE CASCADE;
CREATE INDEX IF NOT EXISTS idx_app_org ON app(organization_id);
