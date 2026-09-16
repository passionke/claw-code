-- Per-apprentice peer MCP token (remote gateway CLAW_MASTER_MCP_TOKEN). Author: kejiqing

ALTER TABLE project_master_link
  ADD COLUMN IF NOT EXISTS apprentice_mcp_token TEXT NOT NULL DEFAULT '';
