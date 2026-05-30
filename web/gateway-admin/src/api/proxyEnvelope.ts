/** Parse /__proxy__ envelope (JSON upstream → `body` object). Author: kejiqing */

export type ProxyEnvelope = {
  ok?: boolean;
  status?: number;
  headers?: Record<string, string>;
  contentType?: string;
  body?: unknown;
  bodyText?: string;
  error?: string;
};

export function upstreamBodyFromEnvelope(wrap: ProxyEnvelope): unknown {
  if (wrap.body !== undefined) return wrap.body;
  const t = wrap.bodyText;
  if (t == null || t === "") return null;
  try {
    return JSON.parse(t) as unknown;
  } catch {
    return t;
  }
}

export function upstreamErrorMessage(wrap: ProxyEnvelope, fallback = "请求失败"): string {
  const b = upstreamBodyFromEnvelope(wrap);
  if (b && typeof b === "object") {
    const o = b as { detail?: unknown; message?: unknown };
    if (o.detail != null) return String(o.detail);
    if (o.message != null) return String(o.message);
  }
  return wrap.bodyText || wrap.error || fallback;
}
