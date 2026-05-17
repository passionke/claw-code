/** Claw Web project (= gateway dsId workspace). Author: kejiqing */

export type ProjectStorageProtocol = "local" | "oss";

/** Project row + resolved storage (no secrets). */
export type ClawWebProject = {
  projectId: string;
  dsId: number;
  tenantId: string | null;
  title: string;
  description: string;
  status: "active" | "archived";
  storageProtocol: ProjectStorageProtocol;
  ossBucket: string | null;
  ossPrefix: string;
  ossEndpoint: string | null;
  ossRegion: string | null;
  createdAtMs: number;
  updatedAtMs: number;
  role?: "owner" | "member" | "viewer";
};

export type ProjectStorageView = {
  protocol: ProjectStorageProtocol;
  /** Canonical URI: oss://bucket/prefix or local://ds_{id}/ */
  uri: string;
  bucket: string | null;
  prefix: string | null;
  endpoint: string | null;
  region: string | null;
  localWorkspaceRel: string;
  ossConfigured: boolean;
};

export function projectIdFromDsId(dsId: number): string {
  return String(dsId);
}

export function parseDsIdFromProjectId(projectId: string): number {
  const n = Number.parseInt(projectId, 10);
  return Number.isFinite(n) && n >= 1 ? n : 1;
}
