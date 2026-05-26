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
  hasMcpStart: boolean;
  mcpStartOk?: boolean;
  mcpStartMessage?: string;
  warnings: string[];
  errors: string[];
  durationMs: number;
  hint: string;
}

export interface McpTestRequest {
  dsId: number;
  serverName: string;
  config: Record<string, unknown>;
  probeMcpStart?: boolean;
}
