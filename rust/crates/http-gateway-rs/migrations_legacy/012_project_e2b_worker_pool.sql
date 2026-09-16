-- Per-project strict worker pool slots (slot_index 0..N-1). Author: kejiqing
ALTER TABLE project_e2b_worker ADD COLUMN IF NOT EXISTS slot_index INT NOT NULL DEFAULT 0;
