/** Build solve_async extraSession from dynamic ds fields. Author: kejiqing */

import {
  CLAW_EXTRA_CLIENT_ORIGIN,
  CLIENT_ORIGIN_GATEWAY_ADMIN,
} from "./clientOrigin";

export function buildExtraSession(fieldValues: Record<string, string>): Record<string, string> {
  const extra: Record<string, string> = {
    tenant_code: "GPOS",
    solution_code: "restaurant",
    biz_type: "BOSS_REPORT",
    [CLAW_EXTRA_CLIENT_ORIGIN]: CLIENT_ORIGIN_GATEWAY_ADMIN,
  };
  for (const [key, raw] of Object.entries(fieldValues)) {
    const k = key.trim();
    if (!k) continue;
    extra[k] = raw;
  }
  return extra;
}
