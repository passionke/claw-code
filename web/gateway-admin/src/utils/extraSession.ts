/** Build solve_async extraSession; org_id keeps user input verbatim (incl. spaces). Author: kejiqing */

export interface ExtraSessionInput {
  storeId: string;
  orgId: string;
}

export function buildExtraSession({ storeId, orgId }: ExtraSessionInput): Record<string, string> {
  const extra: Record<string, string> = {
    tenant_code: "GPOS",
    solution_code: "restaurant",
    biz_type: "BOSS_REPORT",
  };
  const store = storeId.trim();
  if (store) extra.store_id = store;
  if (orgId.length > 0) extra.org_id = orgId;
  return extra;
}
