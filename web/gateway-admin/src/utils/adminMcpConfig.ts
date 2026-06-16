/** Admin MCP token → Cursor / settings `mcpServers` snippet. Author: kejiqing */

export const DEFAULT_ADMIN_MCP_SERVER_NAME = "claw-gateway-admin";

export function slugAdminMcpServerName(name: string): string {
  const slug = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return slug ? `claw-admin-${slug}` : DEFAULT_ADMIN_MCP_SERVER_NAME;
}

export function buildAdminMcpServersJson(
  gatewayBase: string,
  token: string,
  opts?: {
    endpointPath?: string;
    transport?: string;
    serverName?: string;
  }
): string {
  const base = gatewayBase.replace(/\/$/, "");
  const path = opts?.endpointPath || "/v1/admin/mcp";
  const transport = opts?.transport || "streamable-http";
  const serverName = (opts?.serverName || DEFAULT_ADMIN_MCP_SERVER_NAME).trim();
  const config = {
    mcpServers: {
      [serverName]: {
        type: transport,
        url: `${base}${path}`,
        headers: {
          Authorization: `Bearer ${token}`,
        },
      },
    },
  };
  return JSON.stringify(config, null, 2);
}
