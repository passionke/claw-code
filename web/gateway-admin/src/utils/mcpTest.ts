/** MCP connectivity test API. Author: kejiqing */

import { proxyHttp } from "../api/client";
import type { McpTestRequest, McpTestResponse } from "../types/mcpTest";

/** Gateway may omit empty vec fields (`skip_serializing_if`); normalize before render. */
export function normalizeMcpTestResponse(raw: unknown): McpTestResponse {
  const r = (raw && typeof raw === "object" ? raw : {}) as Partial<McpTestResponse>;
  return {
    ok: !!r.ok,
    status: typeof r.status === "string" ? r.status : "unknown",
    serverName: typeof r.serverName === "string" ? r.serverName : "",
    url: r.url,
    transport: r.transport,
    discoverOk: !!r.discoverOk,
    toolCount: typeof r.toolCount === "number" ? r.toolCount : 0,
    toolsSample: Array.isArray(r.toolsSample) ? r.toolsSample : [],
    warnings: Array.isArray(r.warnings) ? r.warnings : [],
    errors: Array.isArray(r.errors) ? r.errors : [],
    durationMs: typeof r.durationMs === "number" ? r.durationMs : 0,
    hint: typeof r.hint === "string" ? r.hint : "",
  };
}

export async function testMcpServer(
  gatewayBase: string,
  req: McpTestRequest
): Promise<McpTestResponse> {
  const raw = await proxyHttp<unknown>(gatewayBase, "POST", "/v1/mcp/test", req);
  return normalizeMcpTestResponse(raw);
}
