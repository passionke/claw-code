import { Button, Input, Space, Table, Typography, message, Alert } from "antd";
import { DeleteOutlined, PlusOutlined } from "@ant-design/icons";
import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import DraftEditingBanner from "../components/DraftEditingBanner";
import { useProjectConfigEditor } from "../hooks/useProjectConfigEditor";

type EnvRow = { key: string; name: string; value: string };

function mapToRows(env: Record<string, string> | undefined): EnvRow[] {
  const entries = Object.entries(env ?? {});
  entries.sort(([a], [b]) => a.localeCompare(b));
  return entries.map(([name, value], i) => ({
    key: `${i}-${name}`,
    name,
    value,
  }));
}

function rowsToMap(rows: EnvRow[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const row of rows) {
    const name = row.name.trim();
    if (!name) continue;
    out[name] = row.value;
  }
  return out;
}

/** Project worker env (create-time only). Author: kejiqing */
export default function EnvPage() {
  const { projId, projectConfig, reloadEditingConfig, saveDraftPatch } = useProjectConfigEditor();
  const [rows, setRows] = useState<EnvRow[]>([]);

  const syncFromConfig = useCallback((cfg: { workerEnvJson?: Record<string, string> }) => {
    setRows(mapToRows(cfg.workerEnvJson));
  }, []);

  const load = useCallback(
    async (quiet?: boolean) => {
      const cfg = await reloadEditingConfig();
      syncFromConfig(cfg);
      if (!quiet) message.success("环境变量已加载");
    },
    [reloadEditingConfig, syncFromConfig]
  );

  useEffect(() => {
    load(true).catch((e) => message.error(String((e as Error).message)));
  }, [projId, load]);

  useEffect(() => {
    if (projectConfig) syncFromConfig(projectConfig);
  }, [projectConfig, syncFromConfig]);

  const addRow = () => {
    setRows((prev) => [
      ...prev,
      { key: `new-${Date.now()}`, name: "", value: "" },
    ]);
  };

  return (
    <div>
      <Typography.Title level={4}>环境变量</Typography.Title>
      <DraftEditingBanner />
      <Alert
        type="info"
        showIcon
        style={{ marginBottom: 12 }}
        message="仅在新建 / 重建 worker 时注入"
        description={
          <>
            保存在 <Typography.Text code>project_config.worker_env_json</Typography.Text>
            。保存后<strong>不会</strong>自动重启现有 worker；请到{" "}
            <Link to="/worker-profile">Worker profile</Link> 使用「重置 worker」后生效。禁止覆盖
            OPENAI_/ANTHROPIC_/CLAW_ 等系统保留键。
          </>
        }
      />
      <Space style={{ marginBottom: 12 }} wrap>
        <Button icon={<PlusOutlined />} onClick={addRow}>
          添加
        </Button>
        <Button onClick={() => load().catch((e) => message.error(String(e)))}>重新加载</Button>
        <Button
          type="primary"
          onClick={async () => {
            const map = rowsToMap(rows);
            const names = Object.keys(map);
            if (names.length !== rows.filter((r) => r.name.trim()).length) {
              message.error("存在重复的 key");
              return;
            }
            if (rows.some((r) => !r.name.trim() && r.value)) {
              message.error("key 不能为空");
              return;
            }
            await saveDraftPatch({ workerEnvJson: map });
            message.success(`已保存环境变量（${names.length} 项）；重建 worker 后生效`);
          }}
        >
          保存
        </Button>
      </Space>
      <Table
        size="small"
        pagination={false}
        rowKey="key"
        dataSource={rows}
        columns={[
          {
            title: "Key",
            dataIndex: "name",
            width: "36%",
            render: (_: unknown, row: EnvRow, index: number) => (
              <Input
                value={row.name}
                placeholder="MY_VAR"
                onChange={(e) => {
                  const v = e.target.value;
                  setRows((prev) =>
                    prev.map((r, i) => (i === index ? { ...r, name: v } : r))
                  );
                }}
              />
            ),
          },
          {
            title: "Value",
            dataIndex: "value",
            render: (_: unknown, row: EnvRow, index: number) => (
              <Input
                value={row.value}
                placeholder="value"
                onChange={(e) => {
                  const v = e.target.value;
                  setRows((prev) =>
                    prev.map((r, i) => (i === index ? { ...r, value: v } : r))
                  );
                }}
              />
            ),
          },
          {
            title: "",
            width: 56,
            render: (_: unknown, __: EnvRow, index: number) => (
              <Button
                type="text"
                danger
                icon={<DeleteOutlined />}
                onClick={() => setRows((prev) => prev.filter((_, i) => i !== index))}
              />
            ),
          },
        ]}
      />
    </div>
  );
}
