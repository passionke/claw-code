-- Human admin accounts + project membership + UI sessions (per cluster).
-- Author: kejiqing

CREATE TABLE IF NOT EXISTS gateway_admin_accounts (
  account_id TEXT PRIMARY KEY,
  cluster_id TEXT NOT NULL,
  username TEXT NOT NULL,
  password_hash TEXT NOT NULL,
  system_role TEXT NOT NULL DEFAULT 'none',
  disabled BOOLEAN NOT NULL DEFAULT FALSE,
  created_at_ms BIGINT NOT NULL,
  updated_at_ms BIGINT NOT NULL,
  CONSTRAINT chk_gateway_admin_accounts_system_role
    CHECK (system_role IN ('system_admin', 'none'))
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_gateway_admin_accounts_cluster_username
  ON gateway_admin_accounts (cluster_id, username);

CREATE INDEX IF NOT EXISTS ix_gateway_admin_accounts_cluster
  ON gateway_admin_accounts (cluster_id);

CREATE TABLE IF NOT EXISTS gateway_admin_project_members (
  cluster_id TEXT NOT NULL,
  account_id TEXT NOT NULL,
  proj_id BIGINT NOT NULL,
  role TEXT NOT NULL DEFAULT 'space_admin',
  created_at_ms BIGINT NOT NULL,
  PRIMARY KEY (cluster_id, account_id, proj_id),
  CONSTRAINT chk_gateway_admin_project_members_role
    CHECK (role IN ('space_admin')),
  CONSTRAINT fk_gateway_admin_project_members_account
    FOREIGN KEY (account_id) REFERENCES gateway_admin_accounts (account_id)
    ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS ix_gateway_admin_project_members_account
  ON gateway_admin_project_members (cluster_id, account_id);

CREATE INDEX IF NOT EXISTS ix_gateway_admin_project_members_proj
  ON gateway_admin_project_members (cluster_id, proj_id);

CREATE TABLE IF NOT EXISTS gateway_admin_sessions (
  session_id TEXT PRIMARY KEY,
  cluster_id TEXT NOT NULL,
  account_id TEXT NOT NULL,
  token_hash TEXT NOT NULL,
  expires_at_ms BIGINT NOT NULL,
  created_at_ms BIGINT NOT NULL,
  CONSTRAINT fk_gateway_admin_sessions_account
    FOREIGN KEY (account_id) REFERENCES gateway_admin_accounts (account_id)
    ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_gateway_admin_sessions_token_hash
  ON gateway_admin_sessions (token_hash);

CREATE INDEX IF NOT EXISTS ix_gateway_admin_sessions_account
  ON gateway_admin_sessions (cluster_id, account_id);
