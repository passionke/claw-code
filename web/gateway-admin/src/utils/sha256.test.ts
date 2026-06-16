import { describe, expect, it } from "vitest";

import { sha256Hex } from "./sha256";

describe("sha256Hex", () => {
  it("matches known SHA-256 vectors", async () => {
    const empty = await sha256Hex(new TextEncoder().encode(""));
    expect(empty).toBe("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");

    const abc = await sha256Hex(new TextEncoder().encode("abc"));
    expect(abc).toBe("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
  });

  it("falls back when crypto.subtle is unavailable", async () => {
    const original = globalThis.crypto;
    Object.defineProperty(globalThis, "crypto", {
      configurable: true,
      value: {},
    });
    try {
      const out = await sha256Hex(new TextEncoder().encode("abc"));
      expect(out).toBe("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    } finally {
      Object.defineProperty(globalThis, "crypto", {
        configurable: true,
        value: original,
      });
    }
  });
});
