import { CloudDownloadOutlined, ReloadOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Form, Input, Space, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type { AgentEngines, GlobalSettingsResponse } from "../../types/globalSettings";

interface EngineRow {
  id: string;
  ref: string;
  digest: string;
}

interface EngineForm {
  rows: EngineRow[];
}

/** Admin: pin only ref/digest for fixed Agent engine IDs. Author: kejiqing */
export default function AgentEnginesPage() {
  const { gatewayBase } = useApp();
  const [form] = Form.useForm<EngineForm>();
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [engineIds, setEngineIds] = useState<string[]>([]);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const response = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      const rows = Object.entries(response.agentEngines?.engines ?? {})
        .map(([id, entry]) => ({ id, ref: entry.ref, digest: entry.digest }))
        .sort((a, b) => a.id.localeCompare(b.id));
      setEngineIds(rows.map((r) => r.id));
      form.setFieldsValue({ rows });
    } finally {
      setLoading(false);
    }
  }, [form, gatewayBase]);

  useEffect(() => {
    void load();
  }, [load]);

  const save = async () => {
    const values = await form.validateFields();
    const body: AgentEngines = { engines: {} };
    for (const row of values.rows ?? []) {
      const id = row.id.trim();
      body.engines[id] = {
        ref: row.ref.trim(),
        digest: row.digest.trim(),
      };
    }

    setSaving(true);
    try {
      const saved = await proxyHttp<AgentEngines>(
        gatewayBase,
        "PUT",
        "/v1/gateway/global-settings/agent-engines",
        body
      );
      const rows = Object.entries(saved.engines)
        .map(([id, entry]) => ({
          id,
          ref: entry.ref,
          digest: entry.digest,
        }))
        .sort((a, b) => a.id.localeCompare(b.id));
      setEngineIds(rows.map((r) => r.id));
      form.setFieldsValue({ rows });
      message.success("Agent 引擎配置已保存；已有 Worker 不会自动重建");
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Space direction="vertical" size="large" style={{ width: "100%" }}>
      <Space style={{ width: "100%", justifyContent: "space-between" }}>
        <Typography.Title level={4} style={{ margin: 0 }}>
          <CloudDownloadOutlined /> Agent 引擎
        </Typography.Title>
        <Button icon={<ReloadOutlined />} loading={loading} onClick={() => void load()}>
          刷新
        </Button>
      </Space>

      <Alert
        type="info"
        showIcon
        message="引擎 ID 固定，仅可改制品定位"
        description="每行对应已接入的 harness 引擎（如 opencode / appserver）。只改 raw tar.gz URL 与 sha256；不可增删引擎。Worker 启动时只安装项目所选的那一个。"
      />

      {engineIds.length === 0 && !loading ? (
        <Alert
          type="warning"
          showIcon
          message="尚未写入 Agent 引擎映射"
          description="请先用 deploy/agent-engines/migrate-existing-config.sh（或等价运维写入）落好固定引擎条目，再在此页更新 URL / digest。"
        />
      ) : null}

      <Card size="small">
        <Form form={form} initialValues={{ rows: [] }}>
          <Form.List name="rows">
            {(fields) => (
              <Space direction="vertical" style={{ width: "100%" }}>
                {fields.map((field) => (
                  <Space key={field.key} align="start" style={{ display: "flex" }}>
                    <Form.Item {...field} name={[field.name, "id"]} hidden>
                      <Input />
                    </Form.Item>
                    <Form.Item label="引擎" style={{ marginBottom: 0 }}>
                      <Typography.Text code style={{ display: "inline-block", width: 140 }}>
                        {engineIds[field.name] ?? "—"}
                      </Typography.Text>
                    </Form.Item>
                    <Form.Item
                      {...field}
                      name={[field.name, "ref"]}
                      label="URL"
                      rules={[{ required: true, message: "请输入 raw tar.gz URL" }]}
                    >
                      <Input placeholder="https://…/engine-v1.tar.gz" style={{ width: 420 }} />
                    </Form.Item>
                    <Form.Item
                      {...field}
                      name={[field.name, "digest"]}
                      label="digest"
                      rules={[{ required: true, message: "请输入 sha256 digest" }]}
                    >
                      <Input placeholder="sha256:…" style={{ width: 340 }} />
                    </Form.Item>
                  </Space>
                ))}
              </Space>
            )}
          </Form.List>
          <Button
            type="primary"
            loading={saving}
            disabled={engineIds.length === 0}
            onClick={() => void save()}
            style={{ marginTop: 16 }}
          >
            保存
          </Button>
        </Form>
      </Card>
    </Space>
  );
}
