/** MCP editor helpers (`mcpServersJson` object map). Author: kejiqing */

export interface McpEditorItem {
  serverName: string;
  configJson: string;
  enabled?: boolean;
}

function readMcpEnabled(cfg: unknown): boolean | undefined {
  if (!cfg || typeof cfg !== "object" || Array.isArray(cfg)) return undefined;
  const enabled = (cfg as Record<string, unknown>).enabled;
  return typeof enabled === "boolean" ? enabled : undefined;
}

function mcpConfigForEditor(cfg: unknown): Record<string, unknown> {
  if (!cfg || typeof cfg !== "object" || Array.isArray(cfg)) return {};
  const out = { ...(cfg as Record<string, unknown>) };
  delete out.enabled;
  return out;
}

export function mcpListFromRecord(
  rec: Record<string, unknown> | undefined
): McpEditorItem[] {
  if (!rec || typeof rec !== "object" || Array.isArray(rec)) return [];
  return Object.keys(rec)
    .sort()
    .map((serverName) => ({
      serverName,
      configJson: JSON.stringify(mcpConfigForEditor(rec[serverName]), null, 2),
      enabled: readMcpEnabled(rec[serverName]),
    }));
}

export function mcpRecordFromList(list: McpEditorItem[]): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const item of list) {
    const name = item.serverName.trim();
    if (!name) continue;
    let cfg: Record<string, unknown> = {};
    try {
      const parsed = JSON.parse(item.configJson || "{}");
      if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
        throw new Error("not object");
      }
      cfg = parsed as Record<string, unknown>;
    } catch {
      throw new Error(`MCP「${name}」配置 JSON 无效`);
    }
    if (item.enabled === false) cfg.enabled = false;
    else delete cfg.enabled;
    out[name] = cfg;
  }
  return out;
}
