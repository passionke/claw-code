/** GET /v1/sessions/.../turns/.../tools. Author: kejiqing */

export interface TurnToolRecord {
  toolUseId: string;
  toolName: string;
  input: unknown;
  output?: string | null;
  isError?: boolean | null;
  inputTruncated?: boolean;
  outputTruncated?: boolean;
}

export interface TurnToolsResponse {
  sessionId: string;
  turnId: string;
  dsId: number;
  userTurnIndex: number;
  tools: TurnToolRecord[];
}
