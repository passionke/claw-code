/** Build solve_async extraSession; org_id keeps user input verbatim (incl. spaces). Author: kejiqing */

import {
  CLAW_EXTRA_CLIENT_ORIGIN,
  CLIENT_ORIGIN_GATEWAY_ADMIN,
} from "./clientOrigin";

export interface ExtraSessionInput {
  storeId: string;
  orgId: string;
}

export function buildExtraSession({ storeId, orgId }: ExtraSessionInput): Record<string, string> {
  const extra: Record<string, string> = {
    tenant_code: "GPOS",
    solution_code: "restaurant",
    biz_type: "BOSS_REPORT",
    [CLAW_EXTRA_CLIENT_ORIGIN]: CLIENT_ORIGIN_GATEWAY_ADMIN,
  };
  const store = storeId.trim();
  if (store) extra.store_id = store;
  // SQLBot 权限门：org_id 传空字符串即可；须显式下发 key。Author: kejiqing
  extra.org_id = orgId;
  return extra;
}
