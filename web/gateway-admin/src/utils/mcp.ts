/** MCP editor helpers (`mcpServersJson` object map). Author: kejiqing */

export interface McpEditorItem {
  serverName: string;
  configJson: string;
}

export function mcpListFromRecord(
  rec: Record<string, unknown> | undefined
): McpEditorItem[] {
  if (!rec || typeof rec !== "object" || Array.isArray(rec)) return [];
  return Object.keys(rec)
    .sort()
    .map((serverName) => ({
      serverName,
      configJson: JSON.stringify(rec[serverName] ?? {}, null, 2),
    }));
}

export function mcpRecordFromList(list: McpEditorItem[]): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const item of list) {
    const name = item.serverName.trim();
    if (!name) continue;
    let cfg: unknown = {};
    try {
      cfg = JSON.parse(item.configJson || "{}");
    } catch {
      throw new Error(`MCP「${name}」配置 JSON 无效`);
    }
    if (typeof cfg !== "object" || cfg === null || Array.isArray(cfg)) {
      throw new Error(`MCP「${name}」配置必须是 JSON 对象`);
    }
    out[name] = cfg;
  }
  return out;
}
