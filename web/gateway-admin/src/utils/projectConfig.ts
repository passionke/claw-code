import { proxyHttp } from "../api/client";
import type { ProjectConfig } from "../types/project";

export function emptyProjectConfig(dsId: number): ProjectConfig {
  return {
    dsId,
    contentRev: "",
    rulesJson: [],
    mcpServersJson: {},
    skillsJson: [],
    allowedToolsJson: [],
    claudeMd: null,
    solvePreflightJson: { kind: "none" },
  };
}

export async function loadProjectConfig(
  gatewayBase: string,
  dsId: number
): Promise<ProjectConfig> {
  try {
    return await proxyHttp<ProjectConfig>(
      gatewayBase,
      "GET",
      `/v1/project/config/${dsId}`
    );
  } catch (e) {
    const msg = String((e as Error).message || e);
    if (msg.includes("no project_config") || msg.includes("404")) {
      return emptyProjectConfig(dsId);
    }
    throw e;
  }
}

export async function putProjectConfigDraft(
  gatewayBase: string,
  dsId: number,
  cfg: ProjectConfig,
  patch: Partial<ProjectConfig>
): Promise<ProjectConfig> {
  const body = {
    rulesJson: patch.rulesJson ?? cfg.rulesJson ?? [],
    mcpServersJson: patch.mcpServersJson ?? cfg.mcpServersJson ?? {},
    skillsJson: patch.skillsJson ?? cfg.skillsJson ?? [],
    allowedToolsJson: patch.allowedToolsJson ?? cfg.allowedToolsJson ?? [],
    claudeMd: patch.claudeMd !== undefined ? patch.claudeMd : cfg.claudeMd,
    gitSyncJson: patch.gitSyncJson !== undefined ? patch.gitSyncJson : cfg.gitSyncJson,
    solvePreflightJson:
      patch.solvePreflightJson !== undefined
        ? patch.solvePreflightJson
        : cfg.solvePreflightJson,
  };
  const r = await proxyHttp<{ activeConfig?: ProjectConfig } & ProjectConfig>(
    gatewayBase,
    "PUT",
    `/v1/project/config/${dsId}`,
    body
  );
  return r.activeConfig ?? r;
}
