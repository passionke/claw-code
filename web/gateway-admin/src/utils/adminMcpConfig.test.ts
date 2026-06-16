import { describe, expect, it } from "vitest";
import {
  buildAdminMcpServersJson,
  DEFAULT_ADMIN_MCP_SERVER_NAME,
  slugAdminMcpServerName,
} from "./adminMcpConfig";

describe("adminMcpConfig", () => {
  it("builds cursor mcpServers with bearer header", () => {
    const json = buildAdminMcpServersJson(
      "http://192.168.9.252:18088/",
      "camt_amt-1_secret",
      { serverName: DEFAULT_ADMIN_MCP_SERVER_NAME }
    );
    const parsed = JSON.parse(json);
    expect(parsed.mcpServers["claw-gateway-admin"]).toEqual({
      type: "streamable-http",
      url: "http://192.168.9.252:18088/v1/admin/mcp",
      headers: { Authorization: "Bearer camt_amt-1_secret" },
    });
  });

  it("slugifies token display name for server key", () => {
    expect(slugAdminMcpServerName("cursor kejiqing")).toBe("claw-admin-cursor-kejiqing");
  });
});
