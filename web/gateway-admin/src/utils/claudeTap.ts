/** claude-tap Live session URL from Admin global-settings clawTap. Author: kejiqing */

import type { ClawTapSettings } from "../types/globalSettings";

export function isValidHttpUrl(href: string): boolean {
  if (!href || href === "#") return false;
  try {
    const u = new URL(href);
    return u.protocol === "http:" || u.protocol === "https:";
  } catch {
    return false;
  }
}

export function tapLiveFromClawTapSettings(
  tap: ClawTapSettings | undefined | null
): { tapLiveBase: string; tapLiveTemplate: string } {
  if (!tap?.configured || !tap.liveBaseUrl) {
    return { tapLiveBase: "", tapLiveTemplate: "" };
  }
  const tapLiveBase = String(tap.liveBaseUrl).replace(/\/$/, "");
  const tapLiveTemplate =
    String(tap.liveSessionUrlTemplate || "").trim() ||
    `${tapLiveBase}/?session={sessionId}`;
  return { tapLiveBase, tapLiveTemplate };
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
