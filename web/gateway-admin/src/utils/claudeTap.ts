/** claude-tap Live session URL from /healthz. Author: kejiqing */

const GATEWAY_PORT_PLACEHOLDER = /\$\{GATEWAY_HOST_PORT[^}]*\}/g;

/** Expand compose literals left in /healthz when gateway container env was not interpolated. */
export function expandComposeGatewayPort(url: string, gatewayPort: string): string {
  if (!url.includes("${")) return url;
  const port = gatewayPort.trim() || "18088";
  let out = url.replace(GATEWAY_PORT_PLACEHOLDER, port);
  while (out.endsWith("}") && out.includes("://")) {
    const opens = (out.match(/\{/g) || []).length;
    const closes = (out.match(/\}/g) || []).length;
    if (closes <= opens) break;
    out = out.slice(0, -1);
  }
  return out;
}

export function portFromGatewayBase(gatewayBase: string): string {
  try {
    const u = new URL(gatewayBase);
    if (u.port) return u.port;
    return u.protocol === "https:" ? "443" : "80";
  } catch {
    return "18088";
  }
}

export function isValidHttpUrl(href: string): boolean {
  if (!href || href === "#") return false;
  try {
    const u = new URL(href);
    return u.protocol === "http:" || u.protocol === "https:";
  } catch {
    return false;
  }
}

export function normalizeClaudeTapFromHealthz(
  tap: {
    publicLiveBaseUrl?: string;
    liveSessionUrlTemplate?: string;
  },
  gatewayBase: string
): { tapLiveBase: string; tapLiveTemplate: string } {
  const port = portFromGatewayBase(gatewayBase);
  const tapLiveBase = expandComposeGatewayPort(
    String(tap.publicLiveBaseUrl || "").replace(/\/$/, ""),
    port
  );
  const tapLiveTemplate = expandComposeGatewayPort(
    String(tap.liveSessionUrlTemplate || ""),
    port
  );
  const template =
    tapLiveTemplate ||
    (tapLiveBase ? `${tapLiveBase}/?session={sessionId}` : "");
  return { tapLiveBase, tapLiveTemplate: template };
}

export function claudeTapSessionUrl(
  sessionId: string,
  tapLiveBase: string,
  tapLiveTemplate: string
): string {
  if (!sessionId) return "";
  const template = tapLiveTemplate.trim();
  const base = tapLiveBase.trim();
  let href = "";
  if (template) {
    href = template.replace("{sessionId}", encodeURIComponent(sessionId));
  } else if (base) {
    try {
      const u = new URL(base);
      u.searchParams.set("session", sessionId);
      href = u.href;
    } catch {
      href = `${base}?session=${encodeURIComponent(sessionId)}`;
    }
  }
  return isValidHttpUrl(href) ? href : "";
}
