import { CloudOutlined, PlusOutlined } from "@ant-design/icons";
import {
  Alert,
  Button,
  Card,
  Divider,
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
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";

interface GitPatRow {
  id: string;
  name: string;
  note?: string;
  createdAtMs: number;
  updatedAtMs: number;
  tokenSet: boolean;
}

interface ActiveLlmConfig {
  name: string;
  baseModelUrl: string;
  modelName: string;
  apiKeySet: boolean;
}

interface GlobalSettingsResponse {
  updatedAtMs: number;
  gitPats: GitPatRow[];
  activeLlmConfig?: ActiveLlmConfig;
  activeLlmAppliedAtMs?: number;
}

function formatMs(ms?: number): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

export default function GlobalSettingsPage() {
  const { gatewayBase } = useApp();
  const [pats, setPats] = useState<GitPatRow[]>([]);
  const [llmConfigured, setLlmConfigured] = useState(false);
  const [activeLlmAppliedAtMs, setActiveLlmAppliedAtMs] = useState<number | undefined>();
  const [loading, setLoading] = useState(false);
  const [savingLlm, setSavingLlm] = useState(false);
  const [patModalOpen, setPatModalOpen] = useState(false);
  const [editingPat, setEditingPat] = useState<GitPatRow | null>(null);
  const [patForm] = Form.useForm();
  const [llmForm] = Form.useForm();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      setPats(r.gitPats || []);
      setActiveLlmAppliedAtMs(r.activeLlmAppliedAtMs);
      const cfg = r.activeLlmConfig;
      setLlmConfigured(!!cfg);
      if (cfg) {
        llmForm.setFieldsValue({
          name: cfg.name,
          baseModelUrl: cfg.baseModelUrl,
          modelName: cfg.modelName,
          apiKey: "",
        });
      } else {
        llmForm.resetFields();
      }
    } finally {
      setLoading(false);
    }
  }, [gatewayBase, llmForm]);

  useEffect(() => {
    load().catch(() => {
      setPats([]);
      setLlmConfigured(false);
    });
  }, [load]);

  const openCreatePat = () => {
    setEditingPat(null);
    patForm.resetFields();
    setPatModalOpen(true);
  };

  const openEditPat = (row: GitPatRow) => {
    setEditingPat(row);
    patForm.setFieldsValue({ name: row.name, note: row.note || "" });
    setPatModalOpen(true);
  };

  const savePat = async () => {
    const v = await patForm.validateFields();
    const body: {
      id?: string;
      name: string;
      note?: string;
      token?: string;
    } = {
      name: (v.name || "").trim(),
      note: (v.note || "").trim() || undefined,
    };
    if (editingPat) {
      body.id = editingPat.id;
      const tok = (v.token || "").trim();
      if (tok) body.token = tok;
    } else {
      const tok = (v.token || "").trim();
      if (!tok) {
        message.error("新建 PAT 必须填写 Token");
        return;
      }
      body.token = tok;
    }
    await proxyHttp(gatewayBase, "POST", "/v1/gateway/global-settings/git-pats", body);
    message.success(editingPat ? "PAT 已更新" : "PAT 已添加");
    setPatModalOpen(false);
    await load();
  };

  const saveLlm = async () => {
    const v = await llmForm.validateFields();
    const name = (v.name || "").trim();
    const baseModelUrl = (v.baseModelUrl || "").trim();
    const modelName = (v.modelName || "").trim();
    const apiKey = (v.apiKey || "").trim();
    if (!name || !baseModelUrl || !modelName) {
      message.error("请填写名称、Base URL 与模型名称");
      return;
    }
    if (!llmConfigured && !apiKey) {
      message.error("首次配置必须填写 API Key");
      return;
    }
    setSavingLlm(true);
    try {
      const body: {
        name: string;
        baseModelUrl: string;
        modelName: string;
        apiKey?: string;
      } = { name, baseModelUrl, modelName };
      if (apiKey) body.apiKey = apiKey;
      await proxyHttp<ActiveLlmConfig>(
        gatewayBase,
        "PUT",
        "/v1/gateway/global-settings/active-llm-config",
        body
      );
      message.success("全局大模型已保存并同步到 worker / claude-tap");
      await load();
    } catch (e) {
      message.error(String(e));
    } finally {
      setSavingLlm(false);
    }
  };

  const clearLlm = async () => {
    await proxyHttp(
      gatewayBase,
      "DELETE",
      "/v1/gateway/global-settings/llm-models/global"
    );
    message.success("已清除全局大模型配置");
    await load();
  };

  const patColumns: ColumnsType<GitPatRow> = [
    { title: "ID", dataIndex: "id", width: 160 },
    { title: "名称", dataIndex: "name", width: 140 },
    {
      title: "备注",
      dataIndex: "note",
      ellipsis: true,
      render: (n: string | undefined) => n || "—",
    },
    {
      title: "Token",
      width: 100,
      render: (_, row) =>
        row.tokenSet ? <Tag color="green">已配置</Tag> : <Tag>未配置</Tag>,
    },
    {
      title: "操作",
      width: 160,
      render: (_, row) => (
        <Space>
          <Button size="small" onClick={() => openEditPat(row)}>
            编辑
          </Button>
          <Popconfirm
            title="删除此 PAT？"
            description="引用该 PAT 的项目 Git 同步将无法推送，直到重新选择。"
            onConfirm={async () => {
              await proxyHttp(
                gatewayBase,
                "DELETE",
                `/v1/gateway/global-settings/git-pats/${encodeURIComponent(row.id)}`
              );
              message.success("已删除");
              await load();
            }}
          >
            <Button size="small" danger>
              删除
            </Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <div>
      <Typography.Title level={4} style={{ marginTop: 0 }}>
        全局配置
      </Typography.Title>

      <Card
        title={
          <Space>
            <CloudOutlined />
            <span>全局大模型</span>
            {llmConfigured ? <Tag color="success">已配置</Tag> : <Tag>未配置</Tag>}
          </Space>
        }
        size="small"
        style={{ marginBottom: 16 }}
        loading={loading}
      >
        <Alert
          type="info"
          showIcon
          style={{ marginBottom: 12 }}
          message="保存即生效（无版本）"
          description={
            <>
              写入 PostgreSQL 单行配置后，网关立即同步到{" "}
              <code>.env</code> 与 <code>.claw/claw-tap-upstream.json</code>。
              小米等需带 <code>/v1</code>，例如{" "}
              <code>https://token-plan-cn.xiaomimimo.com/v1</code>。
              {activeLlmAppliedAtMs ? (
                <> 最近保存：{formatMs(activeLlmAppliedAtMs)}。</>
              ) : null}
            </>
          }
        />
        <Form form={llmForm} layout="vertical">
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
            <Input placeholder="https://api.deepseek.com" />
          </Form.Item>
          <Form.Item
            name="modelName"
            label="模型 ID（发给厂商 API 的 model 字段）"
            rules={[{ required: true, message: "请填写模型 ID" }]}
            extra="小米：mimo-v2.5-pro。保存时会把 MiMo-V2.5-Pro 等展示名自动纠正为厂商 ID。"
          >
            <Input placeholder="mimo-v2.5-pro" />
          </Form.Item>
          <Form.Item
            name="apiKey"
            label={llmConfigured ? "API Key（留空不修改）" : "API Key"}
            rules={llmConfigured ? [] : [{ required: true, message: "请填写 API Key" }]}
          >
            <Input.Password placeholder="sk-..." autoComplete="new-password" />
          </Form.Item>
          <Space>
            <Button type="primary" loading={savingLlm} onClick={() => saveLlm().catch(() => {})}>
              保存
            </Button>
            {llmConfigured ? (
              <Popconfirm title="清除全局大模型配置？" onConfirm={() => clearLlm().catch((e) => message.error(String(e)))}>
                <Button danger>清除</Button>
              </Popconfirm>
            ) : null}
          </Space>
        </Form>
      </Card>

      <Divider style={{ margin: "8px 0 16px" }} />

      <Card
        title="Git PAT"
        size="small"
        extra={
          <Button type="primary" icon={<PlusOutlined />} onClick={openCreatePat}>
            添加 PAT
          </Button>
        }
      >
        <Table
          rowKey="id"
          size="small"
          loading={loading}
          columns={patColumns}
          dataSource={pats}
          pagination={false}
        />
      </Card>

      <Modal
        title={editingPat ? `编辑 PAT · ${editingPat.id}` : "添加 PAT"}
        open={patModalOpen}
        onCancel={() => setPatModalOpen(false)}
        onOk={() => savePat().catch((e) => message.error(String(e)))}
        destroyOnClose
      >
        <Form form={patForm} layout="vertical">
          <Form.Item
            name="name"
            label="名称"
            rules={[{ required: true, message: "请填写名称" }]}
          >
            <Input placeholder="例如 GitLab 主账号" />
          </Form.Item>
          <Form.Item name="note" label="备注">
            <Input.TextArea rows={2} placeholder="可选" />
          </Form.Item>
          <Form.Item
            name="token"
            label={editingPat ? "Token（留空表示不修改）" : "Token"}
            rules={editingPat ? [] : [{ required: true, message: "请填写 Token" }]}
          >
            <Input.Password placeholder="Personal Access Token" />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
