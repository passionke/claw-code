/**
 * Sync local editor state with AppContext projectConfig (draft or effective formal).
 * Author: kejiqing
 */

import { useCallback } from "react";
import { useApp } from "../context/AppContext";
import type { ProjectConfig } from "../types/project";
import { loadProjectConfig, putProjectConfigDraft } from "../utils/projectConfig";

export function useProjectConfigEditor() {
  const { gatewayBase, dsId, projectConfig, refreshProjectConfig, applyProjectConfig } =
    useApp();

  const reloadEditingConfig = useCallback(async (): Promise<ProjectConfig> => {
    if (!gatewayBase) throw new Error("未选择网关");
    const cfg = await loadProjectConfig(gatewayBase, dsId);
    applyProjectConfig(cfg);
    return cfg;
  }, [gatewayBase, dsId, applyProjectConfig]);

  const saveDraftPatch = useCallback(
    async (patch: Partial<ProjectConfig>): Promise<ProjectConfig> => {
      const base = projectConfig ?? (await reloadEditingConfig());
      const cfg = await putProjectConfigDraft(gatewayBase, dsId, base, patch);
      applyProjectConfig(cfg);
      return cfg;
    },
    [gatewayBase, dsId, projectConfig, reloadEditingConfig, applyProjectConfig]
  );

  return {
    gatewayBase,
    dsId,
    projectConfig,
    refreshProjectConfig,
    reloadEditingConfig,
    saveDraftPatch,
    applyProjectConfig,
  };
}
