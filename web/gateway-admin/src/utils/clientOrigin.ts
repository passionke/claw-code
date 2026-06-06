/** Gateway client origin markers (`extraSession._claw_client_origin`). Author: kejiqing */

export const CLAW_EXTRA_CLIENT_ORIGIN = "_claw_client_origin";
export const CLIENT_ORIGIN_GATEWAY_ADMIN = "gateway-admin";
export const HEADER_CLIENT_ORIGIN = "X-Claw-Client-Origin";

/** Only `gateway-admin` is admin-owned; missing origin is external (BFF/product portal). Author: kejiqing */
export function isAdminOrigin(origin?: string | null): boolean {
  return origin === CLIENT_ORIGIN_GATEWAY_ADMIN;
}

export function isExternalOrigin(origin?: string | null): boolean {
  return !isAdminOrigin(origin);
}
