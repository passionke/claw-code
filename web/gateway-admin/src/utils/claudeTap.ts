/** claude-tap session Live viewer URL from Admin global-settings clawTap. Author: kejiqing */

import type { ClawTapSettings } from "../types/globalSettings";

/** Browser Claude Trace UI: claude-tap Live `GET /?session=…` on E2B Host traffic URL. */
export function liveSessionViewerUrlTemplate(liveBaseUrl: string): string {
  const base = String(liveBaseUrl || "").replace(/\/$/, "");
  if (!base) return "";
  return `${base}/?session={sessionId}`;
}

/** JSON API (debug only; Admin TurnCard uses viewer template). */
export function liveSessionTracesUrlTemplate(liveBaseUrl: string): string {
  const base = String(liveBaseUrl || "").replace(/\/$/, "");
  if (!base) return "";
  return `${base}/api/sessions/traces?session={sessionId}`;
}

/** Loose http(s) check; e2b traffic hosts use underscores (`3000-sbx_xxx.domain`). Author: kejiqing */
export function isValidHttpUrl(href: string): boolean {
  if (!href || href === "#") return false;
  try {
    const u = new URL(href);
    return u.protocol === "http:" || u.protocol === "https:";
  } catch {
    return /^https?:\/\/[^/\s?#]+/i.test(href);
  }
}

export function tapLiveFromClawTapSettings(
  tap: ClawTapSettings | undefined | null
): { tapLiveBase: string; tapLiveTemplate: string } {
  if (!tap?.liveBaseUrl) {
    return { tapLiveBase: "", tapLiveTemplate: "" };
  }
  const tapLiveBase = String(tap.liveBaseUrl).replace(/\/$/, "");
  const tapLiveTemplate =
    String(tap.liveSessionUrlTemplate || "").trim() ||
    liveSessionViewerUrlTemplate(tapLiveBase);
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
    href = liveSessionViewerUrlTemplate(base).replace(
      "{sessionId}",
      encodeURIComponent(sessionId)
    );
  }
  return href;
}
