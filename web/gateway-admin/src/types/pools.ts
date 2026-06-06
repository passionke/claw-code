/** GET /v1/pools — shared PG pool registry. Author: kejiqing */

export interface ClawPoolEntry {
  poolId: string;
  advertiseIp: string;
  ssePort: number;
  slotsMax: number;
  slotsMin: number;
  registrationTimeMs: number;
  lastHeartbeatMs: number;
  online: boolean;
  httpBase: string;
}

export interface ListClawPoolsResponse {
  pools: ClawPoolEntry[];
  coLocatedPoolId?: string | null;
}
