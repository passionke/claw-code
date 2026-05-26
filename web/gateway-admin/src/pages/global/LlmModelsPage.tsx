import { CloudOutlined, PlusOutlined } from "@ant-design/icons";
import {
  Alert,
  Button,
  Card,
  Form,
  Input,
  Modal,
  Popconfirm,
  Space,
  Table,
  Tag,
  Typography,
  message,
} from "antd";
import type { ColumnsType } from "antd/es/table";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type { GlobalSettingsResponse, LlmModelRow } from "../../types/globalSettings";

function formatMs(ms?: number): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

export default function LlmModelsPage() {
  const { gatewayBase } = useApp();
  const [models, setModels] = useState<LlmModelRow[]>([]);
  const [activeLlmModelId, setActiveLlmModelId] = useState<string | undefined>();
  const [activeLlmAppliedAtMs, setActiveLlmAppliedAtMs] = useState<number | undefined>();
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [editing, setEditing] = useState<LlmModelRow | null>(null);
  const [form] = Form.useForm();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      setModels(r.llmModels || []);
      setActiveLlmModelId(r.activeLlmModelId);
      setActiveLlmAppliedAtMs(r.activeLlmAppliedAtMs);
    } finally {
      setLoading(false);
    }
  }, [gatewayBase]);

  useEffect(() => {
    load().catch(() => {
      setModels([]);
      setActiveLlmModelId(undefined);
    });
  }, [load]);

  const openCreate = () => {
    setEditing(null);
    form.resetFields();
    setModalOpen(true);
  };

  const openEdit = (row: LlmModelRow) => {
    setEditing(row);
    form.setFieldsValue({
      name: row.name,
      baseModelUrl: row.baseModelUrl,
      modelName: row.modelName,
      apiKey: "",
    });
    setModalOpen(true);
  };

  const saveModel = async () => {
    const v = await form.validateFields();
    const name = (v.name || "").trim();
    const baseModelUrl = (v.baseModelUrl || "").trim();
    const modelName = (v.modelName || "").trim();
    const apiKey = (v.apiKey || "").trim();
    if (!name || !baseModelUrl || !modelName) {
      message.error("请填写名称、Base URL 与模型 ID");
      return;
    }
    if (!editing && !apiKey) {
      message.error("新建模型必须填写 API Key");
      return;
    }
    setSaving(true);
    try {
      const body: {
        id?: string;
        name: string;
        baseModelUrl: string;
        modelName: string;
        apiKey?: string;
      } = { name, baseModelUrl, modelName };
      if (editing) body.id = editing.id;
      if (apiKey) body.apiKey = apiKey;
      await proxyHttp<LlmModelRow>(
        gatewayBase,
        "POST",
        "/v1/gateway/global-settings/llm-models",
        body
      );
      message.success(editing ? "模型已更新" : "模型已添加");
      setModalOpen(false);
      await load();
    } catch (e) {
      message.error(String(e));
    } finally {
      setSaving(false);
    }
  };

  const applyModel = async (row: LlmModelRow) => {
    await proxyHttp(
      gatewayBase,
      "POST",
      `/v1/gateway/global-settings/llm-models/${encodeURIComponent(row.id)}/apply`
    );
    message.success(`已切换为当前模型：${row.name}`);
    await load();
  };

  const deleteModel = async (row: LlmModelRow) => {
    await proxyHttp(
      gatewayBase,
      "DELETE",
      `/v1/gateway/global-settings/llm-models/${encodeURIComponent(row.id)}`
    );
    message.success("已删除");
    await load();
  };

  const columns: ColumnsType<LlmModelRow> = [
    { title: "名称", dataIndex: "name", width: 140 },
    {
      title: "状态",
      width: 90,
      render: (_, row) =>
        row.active || row.id === activeLlmModelId ? (
          <Tag color="success">当前</Tag>
        ) : (
          <Tag>—</Tag>
        ),
    },
    {
      title: "Base URL",
      dataIndex: "baseModelUrl",
      ellipsis: true,
    },
    { title: "模型 ID", dataIndex: "modelName", width: 160, ellipsis: true },
    {
      title: "API Key",
      width: 90,
      render: (_, row) =>
        row.apiKeySet ? <Tag color="green">已配置</Tag> : <Tag>未配置</Tag>,
    },
    {
      title: "操作",
      width: 220,
      render: (_, row) => {
        const isActive = row.active || row.id === activeLlmModelId;
        return (
          <Space wrap>
            <Button size="small" onClick={() => openEdit(row)}>
              编辑
            </Button>
            <Button
              size="small"
              type="primary"
              disabled={isActive}
              onClick={() => applyModel(row).catch((e) => message.error(String(e)))}
            >
              设为当前
            </Button>
            <Popconfirm
              title="删除此模型？"
              description={isActive ? "当前生效模型删除后将无全局 LLM。" : undefined}
              onConfirm={() => deleteModel(row).catch((e) => message.error(String(e)))}
            >
              <Button size="small" danger>
                删除
              </Button>
            </Popconfirm>
          </Space>
        );
      },
    },
  ];

  return (
    <div>
      <Typography.Title level={4} style={{ marginTop: 0 }}>
        模型配置
      </Typography.Title>

      <Card
        title={
          <Space>
            <CloudOutlined />
            <span>大模型列表</span>
            {activeLlmModelId ? (
              <Tag color="success">已设当前</Tag>
            ) : (
              <Tag>未设当前</Tag>
            )}
          </Space>
        }
        size="small"
        extra={
          <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>
            添加模型
          </Button>
        }
        loading={loading}
      >
        <Alert
          type="info"
          showIcon
          style={{ marginBottom: 12 }}
          message="列表管理多条上游；「设为当前」后同步 worker / claude-tap"
          description={
            <>
              保存写入 PostgreSQL；切换当前模型会更新 <code>.env</code> 与{" "}
              <code>.claw/claw-tap-upstream.json</code>。小米等需带{" "}
              <code>/v1</code>，例如{" "}
              <code>https://token-plan-cn.xiaomimimo.com/v1</code>。
              {activeLlmAppliedAtMs ? (
                <> 最近切换：{formatMs(activeLlmAppliedAtMs)}。</>
              ) : null}
            </>
          }
        />
        <Table
          rowKey="id"
          size="small"
          columns={columns}
          dataSource={models}
          pagination={false}
          locale={{ emptyText: "暂无模型，点击「添加模型」" }}
        />
      </Card>

      <Modal
        title={editing ? `编辑模型 · ${editing.name}` : "添加模型"}
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={() => saveModel()}
        confirmLoading={saving}
        destroyOnClose
        width={520}
      >
        <Form form={form} layout="vertical">
          <Form.Item
            name="name"
            label="显示名称"
            rules={[{ required: true, message: "请填写名称" }]}
          >
            <Input placeholder="例如 小米 MiMo 生产" />
          </Form.Item>
          <Form.Item
            name="baseModelUrl"
            label="Base URL"
            rules={[{ required: true, message: "请填写 Base URL" }]}
            extra="上游根地址；若厂商要求 /v1/chat/completions，请填写 https://host/v1"
          >
            <Input placeholder="https://api.deepseek.com/v1" />
          </Form.Item>
          <Form.Item
            name="modelName"
            label="模型 ID（发给厂商 API 的 model 字段）"
            rules={[{ required: true, message: "请填写模型 ID" }]}
            extra="小米：mimo-v2.5-pro"
          >
            <Input placeholder="mimo-v2.5-pro" />
          </Form.Item>
          <Form.Item
            name="apiKey"
            label={editing ? "API Key（留空不修改）" : "API Key"}
            rules={editing ? [] : [{ required: true, message: "请填写 API Key" }]}
          >
            <Input.Password placeholder="sk-..." autoComplete="new-password" />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
