/** Shared Claw Web UI config (browser + server). Author: kejiqing */

export const CLAW_AGENT_ID = "claw";

export function bridgeRunUrl(base?: string): string {
  const root = (base ?? process.env.CLAW_AGUI_BRIDGE_URL ?? "http://127.0.0.1:8090").replace(
    /\/$/,
    "",
  );
  return `${root}/v1/agent/run`;
}

export function gatewayBaseUrl(): string {
  return (process.env.CLAW_GATEWAY_BASE_URL ?? "http://127.0.0.1:8088").replace(/\/$/, "");
}

export function defaultDsId(): number {
  const raw = process.env.CLAW_WEB_DEFAULT_DS_ID ?? "1";
  const n = Number.parseInt(raw, 10);
  return Number.isFinite(n) && n > 0 ? n : 1;
}

export const STORAGE_DS_ID = "claw_web_ds_id";
export const STORAGE_THREAD_ID = "claw_web_thread_id";

/** Legacy single-session key (migrated on load). Prefer conversation store active session. */
export function readStoredThreadId(): string | null {
  if (typeof window === "undefined") return null;
  const id = localStorage.getItem(STORAGE_THREAD_ID);
  return id && id.trim() ? id.trim() : null;
}

export function readStoredDsId(): number {
  if (typeof window === "undefined") return defaultDsId();
  const d = localStorage.getItem(STORAGE_DS_ID);
  if (!d) return defaultDsId();
  const n = Number.parseInt(d, 10);
  return Number.isFinite(n) && n > 0 ? n : defaultDsId();
}
