/** Issue/reuse a project ngmk_ so Chat can call POST /v1/responses. Author: kejiqing */

import { proxyHttp } from "../api/client";

export type PlaygroundNgmk = {
  token: string;
  modelAlias: string;
};

function storageKey(gatewayBase: string, projId: number): string {
  return `claw.playground.ngmk.${gatewayBase.replace(/\/$/, "")}.${projId}`;
}

function readCached(gatewayBase: string, projId: number): PlaygroundNgmk | null {
  try {
    const raw = sessionStorage.getItem(storageKey(gatewayBase, projId));
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<PlaygroundNgmk>;
    if (
      typeof parsed.token === "string" &&
      parsed.token.startsWith("ngmk_") &&
      typeof parsed.modelAlias === "string" &&
      parsed.modelAlias.trim()
    ) {
      return { token: parsed.token, modelAlias: parsed.modelAlias.trim() };
    }
  } catch {
    /* ignore */
  }
  return null;
}

function writeCached(gatewayBase: string, projId: number, value: PlaygroundNgmk): void {
  try {
    sessionStorage.setItem(storageKey(gatewayBase, projId), JSON.stringify(value));
  } catch {
    /* ignore */
  }
}

export function clearPlaygroundNgmk(gatewayBase: string, projId: number): void {
  try {
    sessionStorage.removeItem(storageKey(gatewayBase, projId));
  } catch {
    /* ignore */
  }
}

export async function ensurePlaygroundNgmk(
  gatewayBase: string,
  projId: number
): Promise<PlaygroundNgmk> {
  const cached = readCached(gatewayBase, projId);
  if (cached) return cached;
  const issued = await proxyHttp<{
    token: string;
    entry?: { modelAlias?: string };
  }>(gatewayBase, "POST", `/v1/projects/${projId}/model-api-keys`, {
    name: "playground-chat",
    modelAlias: "agent",
    note: "auto-issued for playground chat stream",
  });
  const token = issued.token;
  if (typeof token !== "string" || !token.startsWith("ngmk_")) {
    throw new Error("签发 ngmk_ 失败");
  }
  const modelAlias = issued.entry?.modelAlias?.trim() || "agent";
  const value = { token, modelAlias };
  writeCached(gatewayBase, projId, value);
  return value;
}
