import { describe, expect, it } from "vitest";
import { routePayload, stripClawOscFrames } from "./parseOscFrames";

function encodeFrame(obj: object): string {
  const json = JSON.stringify(obj);
  const bytes = new TextEncoder().encode(json);
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  const b64 = btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  return `\x1b]1337;Claw;${b64}\x07`;
}

describe("routePayload", () => {
  it("strips OSC frames and parses content delta", () => {
    const frame = encodeFrame({
      ev: "content.delta",
      mime: "text/markdown",
      text: "荷风送晚凉。\n",
    });
    const payload = `prompt>${frame}`;
    const { terminalText, frames } = routePayload(payload);
    expect(terminalText).toBe("prompt>");
    expect(frames).toHaveLength(1);
    expect(frames[0]).toMatchObject({ ev: "content.delta", text: "荷风送晚凉。\n" });
  });

  it("stripClawOscFrames removes embedded frames", () => {
    const frame = encodeFrame({ ev: "status", phase: "done", label: "Done" });
    expect(stripClawOscFrames(`line1${frame}line2`)).toBe("line1line2");
  });
});
