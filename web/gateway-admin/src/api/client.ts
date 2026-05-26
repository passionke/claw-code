/** Playground proxy → gateway JSON API. Author: kejiqing */

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
  body?: unknown
): Promise<T> {
  const res = await fetch("/__proxy__", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    credentials: "same-origin",
    body: JSON.stringify({
      baseUrl: gatewayBase.replace(/\/$/, ""),
      method,
      path,
      body: body === undefined ? null : body,
      headers: {},
    }),
  });
  const wrap = (await res.json().catch(() => ({}))) as {
    ok?: boolean;
    bodyText?: string;
    error?: string;
  };
  if (!wrap.ok) {
    let msg = wrap.bodyText || wrap.error || "请求失败";
    try {
      const j = JSON.parse(wrap.bodyText || "") as { detail?: string };
      if (j?.detail) msg = j.detail;
    } catch {
      /* ignore */
    }
    throw new ApiError(msg);
  }
  try {
    return JSON.parse(wrap.bodyText || "null") as T;
  } catch {
    return wrap.bodyText as T;
  }
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
