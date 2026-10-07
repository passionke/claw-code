/** Build solve_async extraSession from dynamic ds fields. Author: kejiqing */

import {
  CLAW_EXTRA_CLIENT_ORIGIN,
  CLIENT_ORIGIN_GATEWAY_ADMIN,
} from "./clientOrigin";

export function buildExtraSession(fieldValues: Record<string, string>): Record<string, string> {
  // Only client-origin marker + project-defined fields; no hard-coded biz defaults. Author: kejiqing
  const extra: Record<string, string> = {
    [CLAW_EXTRA_CLIENT_ORIGIN]: CLIENT_ORIGIN_GATEWAY_ADMIN,
  };
  for (const [key, raw] of Object.entries(fieldValues)) {
    const k = key.trim();
    if (!k) continue;
    extra[k] = raw;
  }
  return extra;
}
