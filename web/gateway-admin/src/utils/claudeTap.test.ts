/** claude-tap URL helpers. Author: kejiqing */

import { describe, expect, it } from "vitest";
import {
  claudeTapSessionUrl,
  isValidHttpUrl,
  liveSessionViewerUrlTemplate,
  tapLiveFromClawTapSettings,
} from "./claudeTap";

describe("isValidHttpUrl", () => {
  it("accepts e2b traffic host with underscore in sandbox id", () => {
    const href =
      "http://3000-sbx_557b8c5611c2.supone.top/?session=abc";
    expect(isValidHttpUrl(href)).toBe(true);
  });

  it("rejects empty and hash-only", () => {
    expect(isValidHttpUrl("")).toBe(false);
    expect(isValidHttpUrl("#")).toBe(false);
  });
});

describe("claudeTapSessionUrl", () => {
  it("builds Live viewer link for e2b Host traffic base", () => {
    const base = "http://3000-sbx_557b8c5611c2.supone.top";
    const template = liveSessionViewerUrlTemplate(base);
    const href = claudeTapSessionUrl("30fc90bc8991483a97beaf7a861da737", base, template);
    expect(href).toContain("?session=30fc90bc8991483a97beaf7a861da737");
    expect(isValidHttpUrl(href)).toBe(true);
  });
});

describe("tapLiveFromClawTapSettings", () => {
  it("returns live viewer template when configured with fc observe direct traffic", () => {
    const out = tapLiveFromClawTapSettings({
      mode: "local",
      host: "",
      proxyPort: 8080,
      updatedAtMs: 1,
      configured: true,
      liveBaseUrl: "http://3000-sbx_557b8c5611c2.supone.top",
      liveSessionUrlTemplate:
        "http://3000-sbx_557b8c5611c2.supone.top/?session={sessionId}",
    });
    expect(out.tapLiveBase).toBe("http://3000-sbx_557b8c5611c2.supone.top");
    expect(out.tapLiveTemplate).toContain("?session={sessionId}");
  });
});
