import { Button, Input, Space, Typography, message } from "antd";
import { useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";

const { TextArea } = Input;

export default function McpPage() {
  const { gatewayBase, dsId, refreshProjectConfig } = useApp();
  const [status, setStatus] = useState("");
  const [servers, setServers] = useState("{}");

  const load = async () => {
    const data = await proxyHttp(gatewayBase, "GET", `/v1/mcp/injected/${dsId}`);
    setStatus(JSON.stringify(data, null, 2));
  };

  useEffect(() => {
    load().catch((e) => message.error(String((e as Error).message)));
  }, [gatewayBase, dsId]);

  const inject = async (replace: boolean) => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(servers || "{}");
    } catch {
      throw new Error("mcpServers JSON 无效");
    }
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      throw new Error("mcpServers 必须是 JSON 对象");
    }
    const r = await proxyHttp(gatewayBase, "POST", "/v1/mcp/inject", {
      dsId,
      mcpServers: parsed,
      replace,
    });
    setStatus(JSON.stringify(r, null, 2));
    message.success(replace ? "MCP 全量写入临时版" : "MCP 合并写入临时版");
    await refreshProjectConfig();
  };

  return (
    <div>
      <Typography.Title level={4}>Project MCP</Typography.Title>
      <pre style={{ fontSize: 12, maxHeight: 160, overflow: "auto" }}>{status}</pre>
      <TextArea rows={12} value={servers} onChange={(e) => setServers(e.target.value)} />
      <Space style={{ marginTop: 8 }}>
        <Button onClick={() => inject(false)}>追加注入</Button>
        <Button type="primary" onClick={() => inject(true)}>
          全量替换
        </Button>
        <Button onClick={() => load()}>刷新状态</Button>
        <Button
          danger
          onClick={async () => {
            const r = await proxyHttp(gatewayBase, "DELETE", `/v1/mcp/injected/${dsId}`);
            setServers("{}");
            setStatus(JSON.stringify(r, null, 2));
            message.success("已清空临时版 MCP");
            await refreshProjectConfig();
          }}
        >
          清空注入
        </Button>
      </Space>
    </div>
  );
}
