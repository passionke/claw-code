import { proxyHttp } from "../api/client";
import type { ProjectConfig } from "../types/project";

export function emptyProjectConfig(projId: number): ProjectConfig {
  return {
    projId,
    contentRev: "",
    rulesJson: [],
    mcpServersJson: {},
    skillsJson: [],
    allowedToolsJson: [],
    claudeMd: null,
    solvePreflightJson: { kind: "none", steps: [] },
    solveOrchestrationJson: { kind: "single_turn" },
    languagePipelineJson: {},
    extraSessionFieldsJson: [],
    promptLimitsJson: {},
    workerProfileJson: { mode: "strict" },
  };
}

/** Load config for Admin editors: draft when open, else effective formal (server-side). */
export async function loadProjectConfig(
  gatewayBase: string,
  projId: number
): Promise<ProjectConfig> {
  try {
    return await proxyHttp<ProjectConfig>(
      gatewayBase,
      "GET",
      `/v1/project/config/${projId}`
    );
  } catch (e) {
    const msg = String((e as Error).message || e);
    if (msg.includes("no project_config") || msg.includes("404")) {
      return emptyProjectConfig(projId);
    }
    throw e;
  }
}

export async function putProjectConfigDraft(
  gatewayBase: string,
  projId: number,
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
    solveOrchestrationJson:
      patch.solveOrchestrationJson !== undefined
        ? patch.solveOrchestrationJson
        : cfg.solveOrchestrationJson,
    languagePipelineJson:
      patch.languagePipelineJson !== undefined
        ? patch.languagePipelineJson
        : cfg.languagePipelineJson,
    extraSessionFieldsJson:
      patch.extraSessionFieldsJson !== undefined
        ? patch.extraSessionFieldsJson
        : cfg.extraSessionFieldsJson ?? [],
    promptLimitsJson:
      patch.promptLimitsJson !== undefined
        ? patch.promptLimitsJson
        : cfg.promptLimitsJson ?? {},
    workerProfileJson:
      patch.workerProfileJson !== undefined
        ? patch.workerProfileJson
        : cfg.workerProfileJson ?? { mode: "strict" },
  };
  const r = await proxyHttp<{ activeConfig?: ProjectConfig } & ProjectConfig>(
    gatewayBase,
    "PUT",
    `/v1/project/config/${projId}`,
    body
  );
  return r.activeConfig ?? r;
}
