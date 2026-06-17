import type { CdpEvent } from "./types";

const OSC_PREFIX = "\x1b]1337;Claw;";
const OSC_SUFFIX = "\x07";

function decodeBase64Url(input: string): string {
  const padded = input.replace(/-/g, "+").replace(/_/g, "/");
  const padLen = (4 - (padded.length % 4)) % 4;
  const withPad = padded + "=".repeat(padLen);
  const binary = atob(withPad);
  const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0));
  return new TextDecoder("utf-8").decode(bytes);
}

function parseFramePayload(encoded: string): CdpEvent | null {
  try {
    const json = decodeBase64Url(encoded);
    const value = JSON.parse(json) as CdpEvent;
    if (!value || typeof value !== "object" || !("ev" in value)) return null;
    return value;
  } catch {
    return null;
  }
}

/** Split ttyd/xterm payload into terminal bytes and CDP frames. */
export function routePayload(payload: string): { terminalText: string; frames: CdpEvent[] } {
  const frames: CdpEvent[] = [];
  let terminal = "";
  let i = 0;
  while (i < payload.length) {
    const start = payload.indexOf(OSC_PREFIX, i);
    if (start < 0) {
      terminal += payload.slice(i);
      break;
    }
    terminal += payload.slice(i, start);
    const bodyStart = start + OSC_PREFIX.length;
    const end = payload.indexOf(OSC_SUFFIX, bodyStart);
    if (end < 0) {
      break;
    }
    const encoded = payload.slice(bodyStart, end);
    const frame = parseFramePayload(encoded);
    if (frame) frames.push(frame);
    i = end + OSC_SUFFIX.length;
  }
  return { terminalText: terminal, frames };
}

export function stripClawOscFrames(payload: string): string {
  return routePayload(payload).terminalText;
}
