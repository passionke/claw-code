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
  const offline = pool.online ? "" : " (offline)";
  return `${pool.poolId} · ${host}${offline}`;
}

/** Pools with non-empty gatewayBase from claw_pool registration. */
export function poolsWithGateway(
  clusterPools: ListClawPoolsResponse | null
): ClawPoolEntry[] {
  return (clusterPools?.pools ?? []).filter((p) =>
    Boolean((p.gatewayBase || "").trim())
  );
}

export function defaultGatewayFromPools(
  playground: PlaygroundConfig,
  clusterPools: ListClawPoolsResponse | null
): string {
  const registered = poolsWithGateway(clusterPools);
  const co = clusterPools?.coLocatedPoolId?.trim();
  if (co) {
    const self = registered.find((p) => p.poolId === co);
    if (self?.gatewayBase) {
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
  const registered = poolsWithGateway(clusterPools);
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

/** Show picker only when multiple registered pools expose distinct gateway URLs. */
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
