/** `POST /v1/mcp/test` response. Author: kejiqing */

export interface McpTestResponse {
  ok: boolean;
  status: string;
  serverName: string;
  url?: string;
  transport?: string;
  discoverOk: boolean;
  toolCount: number;
  toolsSample: string[];
  warnings: string[];
  errors: string[];
  durationMs: number;
  hint: string;
}

export interface McpTestRequest {
  projId: number;
  serverName: string;
  config: Record<string, unknown>;
}
