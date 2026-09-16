-- Master / observation space / repair runs / scheduled jobs. Author: kejiqing

ALTER TABLE project_config
  ADD COLUMN IF NOT EXISTS project_role TEXT NOT NULL DEFAULT 'normal';

-- Role CHECK is owned by 024: this migrator replays every file on startup, so an
-- older ADD CONSTRAINT here would re-narrow and fail once router/knowledge_base rows exist. Author: kejiqing

CREATE TABLE IF NOT EXISTS project_master_link (
  cluster_id TEXT NOT NULL,
  master_proj_id BIGINT NOT NULL,
  apprentice_proj_id BIGINT NOT NULL,
  observation_proj_id BIGINT NOT NULL,
  orphaned BOOLEAN NOT NULL DEFAULT FALSE,
  created_at_ms BIGINT NOT NULL,
  updated_at_ms BIGINT NOT NULL,
  PRIMARY KEY (cluster_id, master_proj_id, apprentice_proj_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_project_master_link_observation
  ON project_master_link (cluster_id, observation_proj_id);

CREATE TABLE IF NOT EXISTS master_repair_run (
  cluster_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  master_proj_id BIGINT NOT NULL,
  apprentice_proj_id BIGINT NOT NULL,
  observation_proj_id BIGINT NOT NULL,
  master_session_id TEXT,
  master_turn_id TEXT,
  status TEXT NOT NULL DEFAULT 'opened',
  inventory_json JSONB NOT NULL DEFAULT '{"items":[]}'::jsonb,
  baseline_apprentice_content_rev TEXT,
  observation_content_rev_before TEXT,
  observation_content_rev_after TEXT,
  replay_session_ids JSONB NOT NULL DEFAULT '[]'::jsonb,
  analysis_json JSONB NOT NULL DEFAULT '{}'::jsonb,
  promote_status TEXT NOT NULL DEFAULT 'none',
  apprentice_draft_note TEXT,
  created_at_ms BIGINT NOT NULL,
  updated_at_ms BIGINT NOT NULL,
  PRIMARY KEY (cluster_id, run_id)
);

CREATE INDEX IF NOT EXISTS idx_master_repair_run_master
  ON master_repair_run (cluster_id, master_proj_id, created_at_ms DESC);

CREATE TABLE IF NOT EXISTS gateway_scheduled_job (
  cluster_id TEXT NOT NULL,
  job_id TEXT NOT NULL,
  master_proj_id BIGINT NOT NULL,
  schedule_kind TEXT NOT NULL DEFAULT 'daily',
  run_at_hhmm TEXT NOT NULL DEFAULT '02:00',
  weekday INT,
  enabled BOOLEAN NOT NULL DEFAULT TRUE,
  prompt_template TEXT NOT NULL DEFAULT '',
  last_run_at_ms BIGINT,
  last_task_id TEXT,
  last_error TEXT,
  created_at_ms BIGINT NOT NULL,
  updated_at_ms BIGINT NOT NULL,
  PRIMARY KEY (cluster_id, job_id)
);

CREATE INDEX IF NOT EXISTS idx_gateway_scheduled_job_master
  ON gateway_scheduled_job (cluster_id, master_proj_id);
