/** Server: claw_projects + user_projects. Author: kejiqing */

import type { ClawWebProject } from "@/lib/claw-project-types";
import { parseDsIdFromProjectId, projectIdFromDsId } from "@/lib/claw-project-types";
import { storageFieldsForNewProject } from "@/lib/claw-project-storage";
import { withPg } from "@/lib/claw-pg";

const PROJECT_COLS = `p.project_id, p.ds_id, p.tenant_id, p.title, p.description, p.status,
  p.storage_protocol, p.oss_bucket, p.oss_prefix, p.oss_endpoint, p.oss_region,
  p.created_at_ms, p.updated_at_ms`;

function mapProject(row: {
  project_id: string;
  ds_id: number;
  tenant_id: string | null;
  title: string;
  description: string;
  status: string;
  storage_protocol: string;
  oss_bucket: string | null;
  oss_prefix: string;
  oss_endpoint: string | null;
  oss_region: string | null;
  created_at_ms: string;
  updated_at_ms: string;
  role?: string;
}): ClawWebProject {
  return {
    projectId: row.project_id,
    dsId: row.ds_id,
    tenantId: row.tenant_id,
    title: row.title,
    description: row.description,
    status: row.status === "archived" ? "archived" : "active",
    storageProtocol: row.storage_protocol === "oss" ? "oss" : "local",
    ossBucket: row.oss_bucket,
    ossPrefix: row.oss_prefix ?? "",
    ossEndpoint: row.oss_endpoint,
    ossRegion: row.oss_region,
    createdAtMs: Number(row.created_at_ms),
    updatedAtMs: Number(row.updated_at_ms),
    role: row.role as ClawWebProject["role"],
  };
}

export async function getProjectById(projectId: string): Promise<ClawWebProject | null> {
  return withPg(async (client) => {
    const res = await client.query(
      `SELECT project_id, ds_id, tenant_id, title, description, status,
              storage_protocol, oss_bucket, oss_prefix, oss_endpoint, oss_region,
              created_at_ms, updated_at_ms
       FROM claw_projects WHERE project_id = $1`,
      [projectId],
    );
    if (res.rowCount === 0) return null;
    return mapProject(res.rows[0]);
  });
}

export async function listProjectsForUser(userId: string): Promise<ClawWebProject[]> {
  return withPg(async (client) => {
    const res = await client.query(
      `SELECT ${PROJECT_COLS}, up.role
       FROM claw_user_projects up
       JOIN claw_projects p ON p.project_id = up.project_id
       WHERE up.user_id = $1 AND p.status = 'active'
       ORDER BY p.updated_at_ms DESC`,
      [userId],
    );
    return res.rows.map(mapProject);
  });
}

/** Ensure project row + membership; default project_id = str(dsId). Author: kejiqing */
export async function ensureProjectForUser(
  userId: string,
  dsId: number,
  opts?: { title?: string; tenantId?: string | null },
): Promise<ClawWebProject> {
  const projectId = projectIdFromDsId(dsId);
  const title = opts?.title?.trim() || `Workspace ds ${dsId}`;
  const tenantId = opts?.tenantId ?? null;
  const now = Date.now();
  const storage = storageFieldsForNewProject(projectId, tenantId);

  return withPg(async (client) => {
    await client.query(
      `INSERT INTO claw_projects (
         project_id, ds_id, tenant_id, title, description, status,
         storage_protocol, oss_bucket, oss_prefix, oss_endpoint, oss_region,
         created_at_ms, updated_at_ms
       ) VALUES ($1, $2, $3, $4, '', 'active', $5, $6, $7, $8, $9, $10, $10)
       ON CONFLICT (project_id) DO UPDATE SET
         ds_id = EXCLUDED.ds_id,
         title = CASE WHEN EXCLUDED.title <> '' THEN EXCLUDED.title ELSE claw_projects.title END,
         storage_protocol = CASE
           WHEN claw_projects.storage_protocol = 'local' AND EXCLUDED.storage_protocol = 'oss'
           THEN EXCLUDED.storage_protocol
           ELSE claw_projects.storage_protocol
         END,
         oss_bucket = COALESCE(claw_projects.oss_bucket, EXCLUDED.oss_bucket),
         oss_prefix = CASE
           WHEN claw_projects.oss_prefix = '' OR claw_projects.oss_prefix IS NULL
           THEN EXCLUDED.oss_prefix
           ELSE claw_projects.oss_prefix
         END,
         oss_endpoint = COALESCE(claw_projects.oss_endpoint, EXCLUDED.oss_endpoint),
         oss_region = COALESCE(claw_projects.oss_region, EXCLUDED.oss_region),
         updated_at_ms = $10`,
      [
        projectId,
        dsId,
        tenantId,
        title,
        storage.storageProtocol,
        storage.ossBucket,
        storage.ossPrefix,
        storage.ossEndpoint,
        storage.ossRegion,
        now,
      ],
    );
    await client.query(
      `INSERT INTO claw_user_projects (user_id, project_id, role, joined_at_ms)
       VALUES ($1, $2, 'owner', $3)
       ON CONFLICT (user_id, project_id) DO NOTHING`,
      [userId, projectId, now],
    );
    const row = await client.query(
      `SELECT ${PROJECT_COLS}, up.role
       FROM claw_projects p
       JOIN claw_user_projects up ON up.project_id = p.project_id AND up.user_id = $1
       WHERE p.project_id = $2`,
      [userId, projectId],
    );
    if (row.rowCount === 0) {
      throw new Error(`project ensure failed: ${projectId}`);
    }
    return mapProject(row.rows[0]);
  });
}

export async function userHasProjectAccess(userId: string, projectId: string): Promise<boolean> {
  return withPg(async (client) => {
    const res = await client.query(
      `SELECT 1 FROM claw_user_projects WHERE user_id = $1 AND project_id = $2`,
      [userId, projectId],
    );
    return (res.rowCount ?? 0) > 0;
  });
}

export async function assertProjectAccess(userId: string, projectId: string): Promise<ClawWebProject> {
  const dsId = parseDsIdFromProjectId(projectId);
  await ensureProjectForUser(userId, dsId);
  const project = await getProjectById(projectId);
  if (!project) {
    throw new Error(`project not found: ${projectId}`);
  }
  const ok = await userHasProjectAccess(userId, projectId);
  if (!ok) {
    throw new Error(`project access denied: ${projectId}`);
  }
  return project;
}
