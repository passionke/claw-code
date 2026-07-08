/** Normalize gateway base URL for comparisons. Author: kejiqing */

export function normalizeGatewayBase(base: string): string {
  return base.trim().replace(/\/$/, "");
}
