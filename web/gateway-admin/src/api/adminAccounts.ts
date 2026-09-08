/** Admin account / me API helpers. Author: kejiqing */

import { proxyHttp } from "./client";

export interface AdminMe {
  accountId: string;
  username: string;
  systemRole: string;
  systemAdmin: boolean;
  projectIds: number[];
}

export interface AdminAccountRow {
  accountId: string;
  username: string;
  systemRole: string;
  disabled: boolean;
  projectIds: number[];
  createdAtMs: number;
  updatedAtMs: number;
}

export async function fetchGatewayMe(gatewayBase: string): Promise<AdminMe> {
  return proxyHttp<AdminMe>(gatewayBase, "GET", "/v1/admin/auth/me");
}

export async function listAdminAccounts(
  gatewayBase: string
): Promise<AdminAccountRow[]> {
  const r = await proxyHttp<{ accounts: AdminAccountRow[] }>(
    gatewayBase,
    "GET",
    "/v1/admin/accounts"
  );
  return r.accounts || [];
}

export async function createAdminAccount(
  gatewayBase: string,
  body: { username: string; password: string; systemRole?: string }
): Promise<AdminAccountRow> {
  return proxyHttp(gatewayBase, "POST", "/v1/admin/accounts", body);
}

export async function patchAdminAccount(
  gatewayBase: string,
  accountId: string,
  body: { password?: string; systemRole?: string; disabled?: boolean }
): Promise<AdminAccountRow> {
  return proxyHttp(
    gatewayBase,
    "PATCH",
    `/v1/admin/accounts/${encodeURIComponent(accountId)}`,
    body
  );
}

export async function putAccountProject(
  gatewayBase: string,
  accountId: string,
  projId: number
): Promise<void> {
  await proxyHttp(
    gatewayBase,
    "PUT",
    `/v1/admin/accounts/${encodeURIComponent(accountId)}/projects/${projId}`,
    null
  );
}

export async function deleteAccountProject(
  gatewayBase: string,
  accountId: string,
  projId: number
): Promise<void> {
  await proxyHttp(
    gatewayBase,
    "DELETE",
    `/v1/admin/accounts/${encodeURIComponent(accountId)}/projects/${projId}`
  );
}
