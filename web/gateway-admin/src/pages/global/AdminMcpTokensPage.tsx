import { CopyOutlined, PlusOutlined } from "@ant-design/icons";
import {
  Alert,
  Button,
  Card,
  Form,
  Input,
  Modal,
  Popconfirm,
  Select,
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
import type {
  AdminMcpTokenIssueResponse,
  AdminMcpTokenRow,
  GlobalSettingsResponse,
} from "../../types/globalSettings";
import {
  buildAdminMcpServersJson,
  slugAdminMcpServerName,
} from "../../utils/adminMcpConfig";
import { copyToClipboard } from "../../utils/copyToClipboard";

function formatMs(ms?: number | null): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

export default function AdminMcpTokensPage() {
  const { gatewayBase } = useApp();
  const [tokens, setTokens] = useState<AdminMcpTokenRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [issued, setIssued] = useState<AdminMcpTokenIssueResponse | null>(null);
  const [form] = Form.useForm();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      setTokens(r.adminMcpTokens || []);
    } finally {
      setLoading(false);
    }
  }, [gatewayBase]);

  useEffect(() => {
    load().catch(() => setTokens([]));
  }, [load]);

  const issueToken = async () => {
    const v = await form.validateFields();
    const body = {
      name: (v.name || "").trim(),
      kind: v.kind as "temporary" | "permanent",
      note: (v.note || "").trim() || undefined,
    };
    const r = await proxyHttp<AdminMcpTokenIssueResponse>(
      gatewayBase,
      "POST",
      "/v1/gateway/global-settings/admin-mcp-tokens",
      body
    );
    setIssued(r);
    setModalOpen(false);
    form.resetFields();
    await load();
  };

  const copyLabel = async (text: string, label: string) => {
    try {
      await copyToClipboard(text);
      message.success(`${label} 已复制`);
    } catch {
      message.error("复制失败");
    }
  };

  const mcpConfigJson =
    issued &&
    buildAdminMcpServersJson(gatewayBase, issued.token, {
      endpointPath: issued.mcpEndpointPath,
      transport: issued.mcpTransport,
      serverName: slugAdminMcpServerName(issued.entry.name),
    });

  const columns: ColumnsType<AdminMcpTokenRow> = [
    { title: "ID", dataIndex: "id", width: 160 },
    { title: "名称", dataIndex: "name", width: 140 },
    {
      title: "类型",
      dataIndex: "kind",
      width: 100,
      render: (k: string) =>
        k === "permanent" ? <Tag color="blue">永久</Tag> : <Tag>24 小时</Tag>,
    },
    {
      title: "状态",
      width: 100,
      render: (_, row) => {
        if (row.revokedAtMs) return <Tag color="default">已吊销</Tag>;
        if (row.expired) return <Tag color="orange">已过期</Tag>;
        if (row.active) return <Tag color="green">有效</Tag>;
        return <Tag>无效</Tag>;
      },
    },
    {
      title: "过期时间",
      dataIndex: "expiresAtMs",
      width: 180,
      render: (ms: number | undefined, row) =>
        row.kind === "permanent" ? "永不过期" : formatMs(ms),
    },
    {
      title: "最近使用",
      dataIndex: "lastUsedAtMs",
      width: 180,
      render: (ms: number | undefined) => formatMs(ms),
    },
    {
      title: "操作",
      width: 100,
      render: (_, row) => (
        <Popconfirm
          title="吊销此 Admin MCP Token？"
          description="吊销后 Cursor / Agent 将无法再用该 Bearer 连接。"
          disabled={!!row.revokedAtMs}
          onConfirm={() =>
            proxyHttp(
              gatewayBase,
              "DELETE",
              `/v1/gateway/global-settings/admin-mcp-tokens/${encodeURIComponent(row.id)}`
            )
              .then(async () => {
                message.success("已吊销");
                await load();
              })
              .catch((e) => message.error(String(e)))
          }
        >
          <Button size="small" danger disabled={!!row.revokedAtMs}>
            吊销
          </Button>
        </Popconfirm>
      ),
    },
  ];

  return (
    <div>
      <Typography.Title level={4} style={{ marginTop: 0 }}>
        Admin MCP Token
      </Typography.Title>
      <Alert
        type="info"
        showIcon
        style={{ marginBottom: 16 }}
        message="独立鉴权"
        description={
          <>
            用于 Cursor / 运维 Agent 连接网关同端口的{" "}
            <Typography.Text code>/v1/admin/mcp</Typography.Text>（streamable-http）。
            与 Admin 登录会话、以及 solve 用的项目 MCP 配置无关。Token 明文仅在颁发时显示一次。
          </>
        }
      />
      <Card
        size="small"
        extra={
          <Button
            type="primary"
            icon={<PlusOutlined />}
            onClick={() => {
              setIssued(null);
              form.resetFields();
              form.setFieldsValue({ kind: "temporary" });
              setModalOpen(true);
            }}
          >
            颁发 Token
          </Button>
        }
      >
        <Table
          rowKey="id"
          size="small"
          loading={loading}
          columns={columns}
          dataSource={tokens}
          pagination={false}
        />
      </Card>

      <Modal
        title="颁发 Admin MCP Token"
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={() => issueToken().catch((e) => message.error(String(e)))}
        okText="颁发"
      >
        <Form form={form} layout="vertical">
          <Form.Item name="name" label="名称" rules={[{ required: true, message: "请输入名称" }]}>
            <Input placeholder="例如 cursor-kejiqing" />
          </Form.Item>
          <Form.Item
            name="kind"
            label="有效期"
            rules={[{ required: true }]}
            initialValue="temporary"
          >
            <Select
              options={[
                { value: "temporary", label: "24 小时临时" },
                { value: "permanent", label: "永久（直至手动吊销）" },
              ]}
            />
          </Form.Item>
          <Form.Item name="note" label="备注">
            <Input.TextArea rows={2} />
          </Form.Item>
        </Form>
      </Modal>

      <Modal
        title="Token 已颁发（请立即保存）"
        open={!!issued}
        onCancel={() => setIssued(null)}
        footer={[
          <Button key="close" type="primary" onClick={() => setIssued(null)}>
            我已保存
          </Button>,
        ]}
      >
        {issued && mcpConfigJson && (
          <Space direction="vertical" style={{ width: "100%" }} size="middle">
            <Alert type="warning" showIcon message="此 Token 不会再次显示，请立即复制保存。" />
            <div>
              <Typography.Text type="secondary">
                MCP 配置（粘贴到 Cursor <Typography.Text code>~/.cursor/mcp.json</Typography.Text>{" "}
                或合并进现有 <Typography.Text code>mcpServers</Typography.Text>）
              </Typography.Text>
              <Input.TextArea rows={12} readOnly value={mcpConfigJson} />
              <Button
                type="primary"
                icon={<CopyOutlined />}
                style={{ marginTop: 8 }}
                onClick={() => copyLabel(mcpConfigJson, "MCP 配置")}
              >
                复制 MCP 配置
              </Button>
            </div>
          </Space>
        )}
      </Modal>
    </div>
  );
}
