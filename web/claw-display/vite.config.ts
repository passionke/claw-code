import { defineConfig } from "vite";
import { resolve } from "node:path";

/** Library bundle for coding.html / future admin reuse. Author: kejiqing */
export default defineConfig({
  build: {
    lib: {
      entry: resolve(__dirname, "src/index.ts"),
      name: "ClawDisplay",
      formats: ["iife"],
      fileName: () => "claw-display.js",
    },
    cssCodeSplit: false,
    outDir: "dist",
    emptyOutDir: true,
  },
});
