/** Gateway client origin markers (`extraSession._claw_client_origin`). Author: kejiqing */

export const CLAW_EXTRA_CLIENT_ORIGIN = "_claw_client_origin";
export const CLIENT_ORIGIN_GATEWAY_ADMIN = "gateway-admin";
export const HEADER_CLIENT_ORIGIN = "X-Claw-Client-Origin";

export function isAdminOrigin(origin?: string | null): boolean {
  return origin === CLIENT_ORIGIN_GATEWAY_ADMIN;
}
