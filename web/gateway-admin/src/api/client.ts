/** Playground proxy → gateway JSON API. Author: kejiqing */

import {
  type ProxyEnvelope,
  upstreamBodyFromEnvelope,
  upstreamErrorMessage,
} from "./proxyEnvelope";

export class ApiError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ApiError";
  }
}

export async function proxyHttp<T = unknown>(
  gatewayBase: string,
  method: string,
  path: string,
  body?: unknown,
  headers?: Record<string, string>
): Promise<T> {
  const reqHeaders: Record<string, string> = { ...(headers ?? {}) };
  const res = await fetch("/__proxy__", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    credentials: "same-origin",
    body: JSON.stringify({
      baseUrl: gatewayBase.replace(/\/$/, ""),
      method,
      path,
      body: body === undefined ? null : body,
      headers: reqHeaders,
    }),
  });
  // Legacy proxy may forward upstream 204; fetch drops the body on 204.
  if (res.ok && res.status === 204) {
    return null as T;
  }
  const wrap = (await res.json().catch(() => ({}))) as ProxyEnvelope;
  if (!wrap.ok) {
    throw new ApiError(upstreamErrorMessage(wrap));
  }
  return upstreamBodyFromEnvelope(wrap) as T;
}

/** Upload files via proxy: JSON envelope → playground rebuilds multipart to gateway. Author: kejiqing */
export async function proxyUploadFiles<T = unknown>(
  gatewayBase: string,
  path: string,
  files: File[]
): Promise<T> {
  const multipartFiles = await Promise.all(
    files.map(async (f) => {
      const buf = await f.arrayBuffer();
      const bytes = new Uint8Array(buf);
      let binary = "";
      for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]!);
      return {
        field: "file",
        filename: f.name,
        mime: f.type || "application/octet-stream",
        dataBase64: btoa(binary),
      };
    })
  );
  const res = await fetch("/__proxy__", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    credentials: "same-origin",
    body: JSON.stringify({
      baseUrl: gatewayBase.replace(/\/$/, ""),
      method: "POST",
      path,
      multipartFiles,
      headers: {},
    }),
  });
  const wrap = (await res.json().catch(() => ({}))) as ProxyEnvelope;
  if (!wrap.ok) {
    throw new ApiError(upstreamErrorMessage(wrap));
  }
  return upstreamBodyFromEnvelope(wrap) as T;
}

export async function fetchPlaygroundConfig(): Promise<PlaygroundConfig> {
  const r = await fetch("/__config__");
  if (!r.ok) throw new ApiError("无法加载 playground 配置");
  return r.json() as Promise<PlaygroundConfig>;
}

export async function fetchAdminMe(): Promise<{ ok: boolean; user?: string }> {
  const r = await fetch("/__admin_me__", { credentials: "same-origin" });
  return r.json() as Promise<{ ok: boolean; user?: string }>;
}

export async function adminLogin(
  user: string,
  password: string,
  next: string
): Promise<{ ok: boolean; next?: string; error?: string }> {
  const r = await fetch("/__admin_login__", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    credentials: "same-origin",
    body: JSON.stringify({ user, password, next }),
  });
  return r.json() as Promise<{ ok: boolean; next?: string; error?: string }>;
}

export async function adminLogout(): Promise<void> {
  await fetch("/__admin_logout__", {
    method: "POST",
    credentials: "same-origin",
  });
}

export interface PlaygroundConfig {
  defaultGatewayBase: string;
  defaultGatewayLabel?: string;
  gatewayPresets?: { label: string; value: string }[];
}
