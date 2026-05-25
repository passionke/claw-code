import { Button, Checkbox, Col, Row, Space, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import DraftEditingBanner from "../components/DraftEditingBanner";
import { useProjectConfigEditor } from "../hooks/useProjectConfigEditor";
import type { ToolCatalogEntry } from "../types/project";

export default function ToolsPage() {
  const { gatewayBase, dsId, projectConfig, reloadEditingConfig, saveDraftPatch } =
    useProjectConfigEditor();
  const [catalog, setCatalog] = useState<ToolCatalogEntry[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());

  const syncToolsFromConfig = useCallback((cfg: { allowedToolsJson?: string[] }) => {
    const arr = Array.isArray(cfg.allowedToolsJson) ? cfg.allowedToolsJson : [];
    setSelected(new Set(arr));
  }, []);

  const load = useCallback(
    async (quiet?: boolean) => {
      const cat = await proxyHttp<{ tools: ToolCatalogEntry[] }>(
        gatewayBase,
        "GET",
        "/v1/project/tools/catalog"
      );
      setCatalog(cat.tools || []);
      const cfg = await reloadEditingConfig();
      syncToolsFromConfig(cfg);
      if (!quiet) message.success("Tools 配置已加载");
    },
    [gatewayBase, reloadEditingConfig, syncToolsFromConfig]
  );

  useEffect(() => {
    load(true).catch((e) => message.error(String((e as Error).message)));
  }, [gatewayBase, dsId, load]);

  useEffect(() => {
    if (projectConfig) syncToolsFromConfig(projectConfig);
  }, [projectConfig, syncToolsFromConfig]);

  return (
    <div>
      <Typography.Title level={4}>Tools 配置</Typography.Title>
      <DraftEditingBanner />
      <Typography.Paragraph type="secondary">
        仅保存在 <Typography.Text code>project_config.allowed_tools_json</Typography.Text>
        （DB）。勾选即启用；未勾选任何项时 solve 不限制工具。不再读取{" "}
        <Typography.Text code>CLAW_ALLOWED_TOOLS</Typography.Text>。
      </Typography.Paragraph>
      <Space style={{ marginBottom: 12 }}>
        <Button
          onClick={() => {
            setSelected(new Set((catalog || []).map((t) => t.name)));
          }}
        >
          全选
        </Button>
        <Button onClick={() => setSelected(new Set())}>全不选</Button>
        <Button onClick={() => load().catch((e) => message.error(String(e)))}>重新加载</Button>
        <Button
          type="primary"
          onClick={async () => {
            await saveDraftPatch({ allowedToolsJson: [...selected] });
            message.success(`已保存 Tools 到草稿（${selected.size} 项）`);
          }}
        >
          保存到草稿
        </Button>
      </Space>
      <Row gutter={[8, 8]}>
        {(catalog || []).map((t) => {
          const checked = selected.has(t.name);
          return (
            <Col xs={24} sm={12} lg={8} key={t.name}>
              <div
                style={{
                  padding: 8,
                  border: "1px solid #2d3a4d",
                  borderRadius: 6,
                }}
              >
                <Checkbox
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
