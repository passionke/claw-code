/** Project center storage: OSS (S3 API) vs local gateway workspace. Author: kejiqing */

import type { ClawWebProject, ProjectStorageView } from "@/lib/claw-project-types";
import { ossEnabledFromEnv, readOssEnvConfig } from "@/lib/claw-oss-config";

export function defaultOssPrefix(projectId: string, tenantId: string | null): string {
  const tenant = tenantId?.trim() || "_";
  return `claw/projects/${tenant}/${projectId}/`;
}

export function localWorkspaceRel(dsId: number): string {
  return `ds_${dsId}/`;
}

/** Resolve storage view for API (no secrets). Author: kejiqing */
export function resolveProjectStorage(project: ClawWebProject): ProjectStorageView {
  const oss = readOssEnvConfig();
  const useOss = project.storageProtocol === "oss" && oss != null;
  if (useOss && oss) {
    const prefix = project.ossPrefix || defaultOssPrefix(project.projectId, project.tenantId);
    const bucket = project.ossBucket || oss.bucket;
    const endpoint = project.ossEndpoint || oss.endpoint;
    return {
      protocol: "oss",
      uri: `oss://${bucket}/${prefix}`,
      bucket,
      prefix,
      endpoint,
      region: project.ossRegion || oss.region,
      localWorkspaceRel: localWorkspaceRel(project.dsId),
      ossConfigured: true,
    };
  }
  return {
    protocol: "local",
    uri: `local://${localWorkspaceRel(project.dsId)}`,
    bucket: null,
    prefix: null,
    endpoint: null,
    region: null,
    localWorkspaceRel: localWorkspaceRel(project.dsId),
    ossConfigured: oss != null,
  };
}

/** Pick protocol + OSS fields when creating/updating a project row. Author: kejiqing */
export function storageFieldsForNewProject(
  projectId: string,
  tenantId: string | null,
): {
  storageProtocol: "local" | "oss";
  ossBucket: string | null;
  ossPrefix: string;
  ossEndpoint: string | null;
  ossRegion: string | null;
} {
  const oss = readOssEnvConfig();
  if (!oss) {
    return {
      storageProtocol: "local",
      ossBucket: null,
      ossPrefix: "",
      ossEndpoint: null,
      ossRegion: null,
    };
  }
  return {
    storageProtocol: "oss",
    ossBucket: oss.bucket,
    ossPrefix: defaultOssPrefix(projectId, tenantId),
    ossEndpoint: oss.endpoint,
    ossRegion: oss.region,
  };
}

export { ossEnabledFromEnv };
