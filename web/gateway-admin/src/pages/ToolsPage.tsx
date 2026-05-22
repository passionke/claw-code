import { Button, Checkbox, Col, Row, Space, Typography, message } from "antd";
import { useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import type { ToolCatalogEntry } from "../types/project";
import { putProjectConfigDraft } from "../utils/projectConfig";

function matchToolPattern(pattern: string, name: string): boolean {
  if (pattern === name) return true;
  if (!pattern.includes("*")) return false;
  if (pattern.endsWith("*")) return name.startsWith(pattern.slice(0, -1));
  return false;
}

function toolSelectable(name: string, gatewayAllowed: string[]): boolean {
  if (!gatewayAllowed.length) return true;
  return gatewayAllowed.some((p) => matchToolPattern(p, name));
}

export default function ToolsPage() {
  const { gatewayBase, dsId, projectConfig, refreshProjectConfig } = useApp();
  const [catalog, setCatalog] = useState<{
    tools: ToolCatalogEntry[];
    gatewayAllowedTools: string[];
  } | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());

  const load = async () => {
    const cat = await proxyHttp<{
      tools: ToolCatalogEntry[];
      gatewayAllowedTools: string[];
    }>(gatewayBase, "GET", "/v1/project/tools/catalog");
    setCatalog(cat);
    const cfg = await refreshProjectConfig();
    const arr = Array.isArray(cfg.allowedToolsJson) ? cfg.allowedToolsJson : [];
    setSelected(new Set(arr));
    message.success("Tools 配置已加载");
  };

  useEffect(() => {
    load().catch((e) => message.error(String((e as Error).message)));
  }, [gatewayBase, dsId]);

  const gatewayAllowed = catalog?.gatewayAllowedTools || [];
  const emptyMeansAll = selected.size === 0;

  return (
    <div>
      <Typography.Title level={4}>Tools 配置</Typography.Title>
      <Typography.Paragraph type="secondary">
        gatewayAllowedTools:{" "}
        {gatewayAllowed.length ? JSON.stringify(gatewayAllowed) : "（空 = 不限制）"}
      </Typography.Paragraph>
      <Space style={{ marginBottom: 12 }}>
        <Button
          onClick={() => {
            const names = (catalog?.tools || [])
              .filter((t) => toolSelectable(t.name, gatewayAllowed))
              .map((t) => t.name);
            setSelected(new Set(names));
          }}
        >
          全选可选
        </Button>
        <Button onClick={() => setSelected(new Set())}>清空勾选</Button>
        <Button onClick={() => load()}>重新加载</Button>
        <Button
          type="primary"
          onClick={async () => {
            if (!projectConfig) return;
            await putProjectConfigDraft(gatewayBase, dsId, projectConfig, {
              allowedToolsJson: [...selected],
            });
            message.success(`已写入临时版（${selected.size} 项）`);
            await refreshProjectConfig();
          }}
        >
          保存到 project_config
        </Button>
      </Space>
      <Row gutter={[8, 8]}>
        {(catalog?.tools || []).map((t) => {
          const ok = toolSelectable(t.name, gatewayAllowed);
          const checked = !emptyMeansAll && selected.has(t.name);
          return (
            <Col xs={24} sm={12} lg={8} key={t.name}>
              <div
                style={{
                  padding: 8,
                  border: "1px solid #2d3a4d",
                  borderRadius: 6,
                  opacity: ok ? 1 : 0.45,
                }}
              >
                <Checkbox
                  disabled={!ok}
                  checked={checked}
                  onChange={(e) => {
                    const next = new Set(selected);
                    if (e.target.checked) next.add(t.name);
                    else next.delete(t.name);
                    setSelected(next);
                  }}
                >
                  <Typography.Text code>{t.name}</Typography.Text>
                </Checkbox>
                <div style={{ fontSize: 12, color: "#8b9cb3" }}>{t.description}</div>
                <div style={{ fontSize: 10, color: "#8b9cb3" }}>{t.source}</div>
              </div>
            </Col>
          );
        })}
      </Row>
    </div>
  );
}
