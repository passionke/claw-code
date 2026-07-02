import { proxyHttp } from "../api/client";
import type {
  PreflightPluginListResponse,
  PreflightPluginRecord,
  PreflightImplJson,
} from "../types/preflight";

export async function fetchPreflightPlugins(
  gatewayBase: string,
): Promise<PreflightPluginRecord[]> {
  const body = await proxyHttp<PreflightPluginListResponse>(
    gatewayBase,
    "GET",
    "/v1/preflight/plugins",
  );
  return body.plugins ?? [];
}

export async function upsertPreflightPlugin(
  gatewayBase: string,
  pluginId: string,
  payload: {
    displayName: string;
    spiVersion?: string;
    defaultImpl?: PreflightImplJson;
    configSchema?: Record<string, unknown>;
  },
): Promise<PreflightPluginRecord> {
  return proxyHttp<PreflightPluginRecord>(
    gatewayBase,
    "PUT",
    `/v1/preflight/plugins/${encodeURIComponent(pluginId)}`,
    payload,
  );
}
