/** GET /v1/sessions/.../turns/.../tools. Author: kejiqing */

export interface TurnToolRecord {
  toolUseId: string;
  toolName: string;
  /** Legacy API field when camelCase rename missing */
  name?: string;
  input: unknown;
  output?: string | null;
  isError?: boolean | null;
  inputTruncated?: boolean;
  outputTruncated?: boolean;
  sequence?: number;
  startedAtMs?: number | null;
  finishedAtMs?: number | null;
}

export interface TurnToolsResponse {
  sessionId: string;
  turnId: string;
  projId: number;
  userTurnIndex: number;
  tools: TurnToolRecord[];
}
