/**
 * Minimal local runner for quick query test.
 * Author: kejiqing
 */

import { spawn } from "node:child_process";

const sql = process.argv.slice(2).join(" ").trim();
if (!sql) {
  console.error("Usage: npm run run-query -- \"SELECT 1\" ");
  process.exit(1);
}

const child = spawn("node", ["dist/index.js"], {
  stdio: "inherit",
  env: process.env,
});

console.log("doris-mcp started (stdio). Use an MCP client to call doris_query.");
console.log(`example sql: ${sql}`);

child.on("exit", (code) => process.exit(code ?? 0));
