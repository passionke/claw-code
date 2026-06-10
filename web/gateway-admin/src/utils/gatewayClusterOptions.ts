/** Pool registry drives gateway dropdown (poolId + registered gatewayBase). Author: kejiqing */

import type { PlaygroundConfig } from "../api/client";
import type { ClawPoolEntry, ListClawPoolsResponse } from "../types/pools";

export function normalizeGatewayBase(base: string): string {
  return base.trim().replace(/\/$/, "");
}

function poolOptionLabel(
  pool: ClawPoolEntry,
  coLocatedPoolId: string | null | undefined
): string {
  let host = pool.advertiseIp;
  const gw = normalizeGatewayBase(pool.gatewayBase || "");
  if (gw) {
    try {
      host = new URL(gw).host;
    } catch {
      host = gw.replace(/^https?:\/\//, "");
    }
  }
  if (pool.poolId === coLocatedPoolId?.trim()) {
    return `本机 · ${pool.poolId}`;
  }
  return `${pool.poolId} · ${host}`;
}

/** Pools with non-empty gatewayBase from claw_pool registration. */
export function poolsWithGateway(
  clusterPools: ListClawPoolsResponse | null,
  onlineOnly = false
): ClawPoolEntry[] {
  return (clusterPools?.pools ?? []).filter(
    (p) =>
      Boolean((p.gatewayBase || "").trim()) && (!onlineOnly || p.online)
  );
}

export function defaultGatewayFromPools(
  playground: PlaygroundConfig,
  clusterPools: ListClawPoolsResponse | null
): string {
  const registered = poolsWithGateway(clusterPools, true);
  const co = clusterPools?.coLocatedPoolId?.trim();
  if (co) {
    const self = registered.find((p) => p.poolId === co);
    if (self?.gatewayBase) {
      // Co-located Admin/playground: browser must use loopback (pool registry LAN IP may be stale). kejiqing
      const def = playground.defaultGatewayBase?.trim();
      if (def) {
        return normalizeGatewayBase(def);
      }
      return normalizeGatewayBase(self.gatewayBase);
    }
  }
  if (registered.length === 1 && registered[0].gatewayBase) {
    return normalizeGatewayBase(registered[0].gatewayBase);
  }
  const def = playground.defaultGatewayBase?.trim();
  return def ? normalizeGatewayBase(def) : "";
}

export function buildGatewayOptions(params: {
  playground: PlaygroundConfig;
  clusterPools: ListClawPoolsResponse | null;
  gatewayBase: string;
  gatewayImageTag: string;
}): { label: string; value: string; poolId: string }[] {
  const { playground, clusterPools, gatewayBase, gatewayImageTag } = params;
  const registered = poolsWithGateway(clusterPools, true);
  const tagSuffix =
    gatewayImageTag && gatewayBase ? ` · ${gatewayImageTag}` : "";
  const labelFor = (baseLabel: string, value: string) =>
    normalizeGatewayBase(value) === normalizeGatewayBase(gatewayBase)
      ? baseLabel + tagSuffix
      : baseLabel;

  if (registered.length === 0) {
    const def = normalizeGatewayBase(playground.defaultGatewayBase || "");
    if (!def) return [];
    return [
      {
        poolId: clusterPools?.coLocatedPoolId?.trim() || "",
        value: def,
        label: labelFor(
          playground.defaultGatewayLabel || `本机 · ${new URL(def).host}`,
          def
        ),
      },
    ];
  }

  const co = clusterPools?.coLocatedPoolId;
  const seen = new Set<string>();
  const out: { label: string; value: string; poolId: string }[] = [];
  const sorted = [...registered].sort((a, b) => {
    if (a.poolId === co) return -1;
    if (b.poolId === co) return 1;
    return a.poolId.localeCompare(b.poolId);
  });

  for (const pool of sorted) {
    const v = normalizeGatewayBase(pool.gatewayBase!.trim());
    if (!v || seen.has(v)) continue;
    seen.add(v);
    out.push({
      poolId: pool.poolId,
      value: v,
      label: labelFor(poolOptionLabel(pool, co), v),
    });
  }
  return out;
}

export function allGatewayOptionValues(
  playground: PlaygroundConfig,
  clusterPools: ListClawPoolsResponse | null
): string[] {
  return buildGatewayOptions({
    playground,
    clusterPools,
    gatewayBase: "",
    gatewayImageTag: "",
  }).map((o) => o.value);
}

/** Turn route: `claw_pool.gateway_base` for `pool_id`, else UI fallback. Author: kejiqing */
export function gatewayBaseForPoolId(
  poolId: string | null | undefined,
  clusterPools: ListClawPoolsResponse | null,
  fallbackGatewayBase: string
): string {
  const pid = (poolId || "").trim();
  if (!pid) return normalizeGatewayBase(fallbackGatewayBase);
  const pool = (clusterPools?.pools ?? []).find((p) => p.poolId === pid);
  const gw = pool?.gatewayBase?.trim();
  if (gw) return normalizeGatewayBase(gw);
  return normalizeGatewayBase(fallbackGatewayBase);
}

/** Show picker when multiple online pools expose distinct gateway URLs. */
export function shouldShowGatewayPicker(
  playground: PlaygroundConfig,
  clusterPools: ListClawPoolsResponse | null
): boolean {
  return (
    buildGatewayOptions({
      playground,
      clusterPools,
      gatewayBase: "",
      gatewayImageTag: "",
    }).length > 1
  );
}
