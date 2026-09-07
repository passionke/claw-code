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

/** Issue / revoke own Admin MCP tokens bound to accountId. Author: kejiqing */
export default function MyMcpTokensPage() {
  const { gatewayBase } = useApp();
  const [tokens, setTokens] = useState<AdminMcpTokenRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [issued, setIssued] = useState<AdminMcpTokenIssueResponse | null>(null);
  const [form] = Form.useForm();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<{ tokens: AdminMcpTokenRow[] }>(
        gatewayBase,
        "GET",
        "/v1/admin/me/mcp-tokens"
      );
      setTokens(r.tokens || []);
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
      "/v1/admin/me/mcp-tokens",
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
    { title: "名称", dataIndex: "name" },
    {
      title: "类型",
      dataIndex: "kind",
      width: 100,
      render: (k: string) => <Tag>{k}</Tag>,
    },
    {
      title: "状态",
      width: 100,
      render: (_, row) =>
        row.active ? <Tag color="green">active</Tag> : <Tag>inactive</Tag>,
    },
    {
      title: "创建",
      dataIndex: "createdAtMs",
      width: 160,
      render: formatMs,
    },
    {
      title: "操作",
      width: 100,
      render: (_, row) => (
        <Popconfirm
          title="吊销此 token？"
          onConfirm={async () => {
            await proxyHttp(
              gatewayBase,
              "DELETE",
              `/v1/admin/me/mcp-tokens/${encodeURIComponent(row.id)}`
            );
            message.success("已吊销");
            await load();
          }}
        >
          <Button size="small" danger>
            吊销
          </Button>
        </Popconfirm>
      ),
    },
  ];

  return (
    <div style={{ padding: 16 }}>
      <Space style={{ marginBottom: 16 }} wrap>
        <Typography.Title level={4} style={{ margin: 0 }}>
          我的 MCP Token
        </Typography.Title>
        <Button
          type="primary"
          icon={<PlusOutlined />}
          onClick={() => setModalOpen(true)}
        >
          颁发
        </Button>
      </Space>
      <Alert
        type="info"
        showIcon
        style={{ marginBottom: 16 }}
        message="Token 绑定当前账号权限：空间管理员只能管理已加入的 projId；明文仅颁发时显示一次。"
      />
      <Table
        rowKey="id"
        loading={loading}
        columns={columns}
        dataSource={tokens}
        pagination={false}
        size="middle"
      />

      <Modal
        title="颁发我的 Admin MCP Token"
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={() => void issueToken().catch((e) => message.error(String(e)))}
        destroyOnClose
      >
        <Form form={form} layout="vertical" initialValues={{ kind: "permanent" }}>
          <Form.Item name="name" label="名称" rules={[{ required: true }]}>
            <Input />
          </Form.Item>
          <Form.Item name="kind" label="类型" rules={[{ required: true }]}>
            <Select
              options={[
                { value: "permanent", label: "永久（直到吊销）" },
                { value: "temporary", label: "临时（24h）" },
              ]}
            />
          </Form.Item>
          <Form.Item name="note" label="备注">
            <Input.TextArea rows={2} />
          </Form.Item>
        </Form>
      </Modal>

      <Modal
        title="Token 已颁发（仅此一次）"
        open={!!issued}
        onCancel={() => setIssued(null)}
        footer={[
          <Button key="close" onClick={() => setIssued(null)}>
            关闭
          </Button>,
        ]}
      >
        {issued && (
          <Space direction="vertical" style={{ width: "100%" }} size="middle">
            <Alert type="warning" message="请立即复制，之后无法再查看明文。" />
            <Card size="small" title="Bearer token">
              <Typography.Paragraph copyable={{ text: issued.token }} code>
                {issued.token}
              </Typography.Paragraph>
              <Button
                icon={<CopyOutlined />}
                onClick={() => void copyLabel(issued.token, "Token")}
              >
                复制 token
              </Button>
            </Card>
            {mcpConfigJson && (
              <Card size="small" title="mcpServers JSON">
                <Typography.Paragraph>
                  <pre style={{ whiteSpace: "pre-wrap", margin: 0 }}>
                    {mcpConfigJson}
                  </pre>
                </Typography.Paragraph>
                <Button
                  icon={<CopyOutlined />}
                  onClick={() => void copyLabel(mcpConfigJson, "MCP JSON")}
                >
                  复制 mcp.json 片段
                </Button>
              </Card>
            )}
          </Space>
        )}
      </Modal>
    </div>
  );
}
