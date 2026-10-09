import {
  CloudDownloadOutlined,
  MinusCircleOutlined,
  PlusOutlined,
  ReloadOutlined,
} from "@ant-design/icons";
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

/** Admin: maintain the dynamic Agent engine artifact map. Author: kejiqing */
export default function AgentEnginesPage() {
  const { gatewayBase } = useApp();
  const [form] = Form.useForm<EngineForm>();
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const response = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      const rows = Object.entries(response.agentEngines?.engines ?? {}).map(
        ([id, entry]) => ({ id, ref: entry.ref, digest: entry.digest })
      );
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
      if (body.engines[id]) {
        message.error(`Agent engine ID 重复：${id}`);
        return;
      }
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
      form.setFieldsValue({
        rows: Object.entries(saved.engines).map(([id, entry]) => ({
          id,
          ref: entry.ref,
          digest: entry.digest,
        })),
      });
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
        message="每个 Agent 引擎独立发布"
        description="填写 raw tar.gz URL 与 upload-raw.sh 输出的 sha256。Worker 启动时只安装项目选择的一个引擎。"
      />

      <Card size="small">
        <Form form={form} initialValues={{ rows: [] }}>
          <Form.List name="rows">
            {(fields, { add, remove }) => (
              <Space direction="vertical" style={{ width: "100%" }}>
                {fields.map((field) => (
                  <Space key={field.key} align="start" style={{ display: "flex" }}>
                    <Form.Item
                      {...field}
                      name={[field.name, "id"]}
                      rules={[{ required: true, message: "请输入 engine ID" }]}
                    >
                      <Input placeholder="engine ID" style={{ width: 180 }} />
                    </Form.Item>
                    <Form.Item
                      {...field}
                      name={[field.name, "ref"]}
                      rules={[{ required: true, message: "请输入 raw tar.gz URL" }]}
                    >
                      <Input placeholder="https://…/engine-v1.tar.gz" style={{ width: 420 }} />
                    </Form.Item>
                    <Form.Item
                      {...field}
                      name={[field.name, "digest"]}
                      rules={[{ required: true, message: "请输入 sha256 digest" }]}
                    >
                      <Input placeholder="sha256:…" style={{ width: 340 }} />
                    </Form.Item>
                    <Button
                      danger
                      type="text"
                      icon={<MinusCircleOutlined />}
                      onClick={() => remove(field.name)}
                    />
                  </Space>
                ))}
                <Button type="dashed" icon={<PlusOutlined />} onClick={() => add()}>
                  添加 Agent 引擎
                </Button>
              </Space>
            )}
          </Form.List>
          <Button type="primary" loading={saving} onClick={() => void save()} style={{ marginTop: 16 }}>
            保存
          </Button>
        </Form>
      </Card>
    </Space>
  );
}
