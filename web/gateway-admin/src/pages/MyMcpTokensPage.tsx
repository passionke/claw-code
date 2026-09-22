import { CopyOutlined, PlusOutlined } from "@ant-design/icons";
import {
  Alert,
  Button,
  Card,
  Form,
  Input,
  Modal,
  Popconfirm,
  Segmented,
  Select,
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
import type {
  AdminMcpTokenIssueResponse,
  AdminMcpTokenRow,
} from "../types/globalSettings";
import {
  buildAdminMcpServersJson,
  slugAdminMcpServerName,
} from "../utils/adminMcpConfig";
import { copyToClipboard } from "../utils/copyToClipboard";

type TokenKind = "mcp" | "login" | "api";

interface LoginSessionRow {
  sessionId: string;
  createdAtMs: number;
  expiresAtMs: number;
  current: boolean;
}

interface ModelApiKeyRow {
  id: string;
  projId: number;
  modelAlias: string;
  name: string;
  note: string;
  tokenPrefix: string;
  status: string;
  createdAtMs: number;
  revokedAtMs?: number | null;
  lastUsedAtMs?: number | null;
}

type Issued =
  | { kind: "mcp"; body: AdminMcpTokenIssueResponse }
  | { kind: "login"; token: string; sessionId: string; expiresAtMs: number }
  | { kind: "api"; token: string; modelAlias: string };

function formatMs(ms?: number | null): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

/** Account MCP (`camt_`), login session (`cass_`), and project call key (`ngmk_`). Author: kejiqing */
export default function MyMcpTokensPage() {
  const { gatewayBase, projId } = useApp();
  const [kind, setKind] = useState<TokenKind>("mcp");
  const [mcpTokens, setMcpTokens] = useState<AdminMcpTokenRow[]>([]);
  const [sessions, setSessions] = useState<LoginSessionRow[]>([]);
  const [apiKeys, setApiKeys] = useState<ModelApiKeyRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [issued, setIssued] = useState<Issued | null>(null);
  const [form] = Form.useForm();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      if (kind === "mcp") {
        const r = await proxyHttp<{ tokens: AdminMcpTokenRow[] }>(
          gatewayBase,
          "GET",
          "/v1/admin/me/mcp-tokens"
        );
        setMcpTokens(r.tokens || []);
      } else if (kind === "login") {
        const r = await proxyHttp<{ sessions: LoginSessionRow[] }>(
          gatewayBase,
          "GET",
          "/v1/admin/me/sessions"
        );
        setSessions(r.sessions || []);
      } else {
        const r = await proxyHttp<{ keys: ModelApiKeyRow[] }>(
          gatewayBase,
          "GET",
          `/v1/projects/${projId}/model-api-keys`
        );
        setApiKeys(r.keys || []);
      }
    } finally {
      setLoading(false);
    }
  }, [gatewayBase, kind, projId]);

  useEffect(() => {
    load().catch(() => {
      if (kind === "mcp") setMcpTokens([]);
      else if (kind === "login") setSessions([]);
      else setApiKeys([]);
    });
  }, [load, kind]);

  const copyLabel = async (text: string, label: string) => {
    try {
      await copyToClipboard(text);
      message.success(`${label} 已复制`);
    } catch {
      message.error("复制失败");
    }
  };

  const issueToken = async () => {
    if (kind === "login") {
      const r = await proxyHttp<{
        token: string;
        sessionId: string;
        expiresAtMs: number;
      }>(gatewayBase, "POST", "/v1/admin/me/sessions");
      setIssued({
        kind: "login",
        token: r.token,
        sessionId: r.sessionId,
        expiresAtMs: r.expiresAtMs,
      });
    } else if (kind === "api") {
      const v = await form.validateFields();
      const modelAlias = (v.modelAlias || "agent").trim() || "agent";
      const r = await proxyHttp<{ token: string; entry: ModelApiKeyRow }>(
        gatewayBase,
        "POST",
        `/v1/projects/${projId}/model-api-keys`,
        {
          name: (v.name || "").trim(),
          modelAlias,
          note: (v.note || "").trim() || undefined,
        }
      );
      setIssued({ kind: "api", token: r.token, modelAlias });
    } else {
      const v = await form.validateFields();
      const r = await proxyHttp<AdminMcpTokenIssueResponse>(
        gatewayBase,
        "POST",
        "/v1/admin/me/mcp-tokens",
        {
          name: (v.name || "").trim(),
          kind: v.kind as "temporary" | "permanent",
          note: (v.note || "").trim() || undefined,
        }
      );
      setIssued({ kind: "mcp", body: r });
    }
    setModalOpen(false);
    form.resetFields();
    await load();
  };

  const mcpConfigJson =
    issued?.kind === "mcp"
      ? buildAdminMcpServersJson(gatewayBase, issued.body.token, {
          endpointPath: issued.body.mcpEndpointPath,
          transport: issued.body.mcpTransport,
          serverName: slugAdminMcpServerName(issued.body.entry.name),
        })
      : null;

  const issuedToken =
    issued?.kind === "mcp"
      ? issued.body.token
      : issued?.kind === "login" || issued?.kind === "api"
        ? issued.token
        : "";

  const mcpColumns: ColumnsType<AdminMcpTokenRow> = [
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
      width: 180,
      render: formatMs,
    },
    {
      title: "操作",
      width: 100,
      render: (_, row) => (
        <Popconfirm
          title="吊销此 MCP token？"
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

  const loginColumns: ColumnsType<LoginSessionRow> = [
    { title: "会话", dataIndex: "sessionId", width: 280 },
    {
      title: "状态",
      width: 100,
      render: (_, row) => (row.current ? <Tag color="blue">当前登录</Tag> : <Tag>有效</Tag>),
    },
    {
      title: "创建",
      dataIndex: "createdAtMs",
      width: 180,
      render: formatMs,
    },
    {
      title: "过期",
      dataIndex: "expiresAtMs",
      width: 180,
      render: formatMs,
    },
    {
      title: "操作",
      width: 100,
      render: (_, row) => (
        <Popconfirm
          title={row.current ? "这是当前浏览器登录，吊销后需要重新登录" : "吊销此登录 token？"}
          onConfirm={async () => {
            await proxyHttp(
              gatewayBase,
              "DELETE",
              `/v1/admin/me/sessions/${encodeURIComponent(row.sessionId)}`
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

  const apiColumns: ColumnsType<ModelApiKeyRow> = [
    { title: "ID", dataIndex: "id", width: 140 },
    { title: "名称", dataIndex: "name" },
    { title: "modelAlias", dataIndex: "modelAlias", width: 120 },
    { title: "前缀", dataIndex: "tokenPrefix", width: 140 },
    {
      title: "状态",
      width: 100,
      render: (_, row) =>
        row.status === "active" && !row.revokedAtMs ? (
          <Tag color="green">active</Tag>
        ) : (
          <Tag>revoked</Tag>
        ),
    },
    {
      title: "创建",
      dataIndex: "createdAtMs",
      width: 180,
      render: formatMs,
    },
    {
      title: "操作",
      width: 100,
      render: (_, row) => (
        <Popconfirm
          title="吊销此调用 token？"
          onConfirm={async () => {
            await proxyHttp(
              gatewayBase,
              "DELETE",
              `/v1/projects/${projId}/model-api-keys/${encodeURIComponent(row.id)}`
            );
            message.success("已吊销");
            await load();
          }}
        >
          <Button size="small" danger disabled={row.status !== "active" || !!row.revokedAtMs}>
            吊销
          </Button>
        </Popconfirm>
      ),
    },
  ];

  const hint =
    kind === "mcp"
      ? "MCP：绑定当前账号的 camt_，用于连接 /v1/admin/mcp。明文只在签发时显示一次。"
      : kind === "login"
        ? "登录：与网页登录同一套 cass_ 会话，Bearer 可访问管理接口，有效期 7 天。新签发不会替换当前浏览器登录。"
        : `接口调用：绑定当前项目 #${projId} 的 ngmk_，用于 /v1/responses 与 /v1/chat/completions。请求里的 model 填 modelAlias。`;

  return (
    <div style={{ padding: 16 }}>
      <Space style={{ marginBottom: 16 }} wrap>
        <Typography.Title level={4} style={{ margin: 0 }}>
          我的 TOKEN
        </Typography.Title>
        <Segmented
          value={kind}
          onChange={(v) => setKind(v as TokenKind)}
          options={[
            { label: "MCP", value: "mcp" },
            { label: "登录", value: "login" },
            { label: "接口调用", value: "api" },
          ]}
        />
        <Button
          type="primary"
          icon={<PlusOutlined />}
          onClick={() => {
            setIssued(null);
            form.resetFields();
            form.setFieldsValue(
              kind === "mcp"
                ? { kind: "permanent" }
                : kind === "api"
                  ? { modelAlias: "agent" }
                  : {}
            );
            setModalOpen(true);
          }}
        >
          签发
        </Button>
      </Space>
      <Alert type="info" showIcon style={{ marginBottom: 16 }} message={hint} />
      {kind === "mcp" ? (
        <Table
          rowKey="id"
          loading={loading}
          columns={mcpColumns}
          dataSource={mcpTokens}
          pagination={false}
          size="middle"
        />
      ) : null}
      {kind === "login" ? (
        <Table
          rowKey="sessionId"
          loading={loading}
          columns={loginColumns}
          dataSource={sessions}
          pagination={false}
          size="middle"
        />
      ) : null}
      {kind === "api" ? (
        <Table
          rowKey="id"
          loading={loading}
          columns={apiColumns}
          dataSource={apiKeys}
          pagination={false}
          size="middle"
        />
      ) : null}

      <Modal
        title={
          kind === "mcp" ? "签发 MCP token" : kind === "login" ? "签发登录 token" : "签发接口调用 token"
        }
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={() => void issueToken().catch((e) => message.error(String(e)))}
        destroyOnClose
      >
        {kind === "login" ? (
          <Typography.Paragraph style={{ marginBottom: 0 }}>
            签发一个新的登录 token（cass_），有效期 7 天。当前浏览器登录保持不变。
          </Typography.Paragraph>
        ) : (
          <Form
            form={form}
            layout="vertical"
            initialValues={kind === "mcp" ? { kind: "permanent" } : { modelAlias: "agent" }}
          >
            <Form.Item name="name" label="名称" rules={[{ required: true, message: "请填写名称" }]}>
              <Input />
            </Form.Item>
            {kind === "mcp" ? (
              <Form.Item name="kind" label="类型" rules={[{ required: true }]}>
                <Select
                  options={[
                    { value: "permanent", label: "永久（直到吊销）" },
                    { value: "temporary", label: "临时（24h）" },
                  ]}
                />
              </Form.Item>
            ) : (
              <Form.Item
                name="modelAlias"
                label="modelAlias"
                rules={[{ required: true, message: "请填写 modelAlias" }]}
                extra="调用时请求体 model 必须与此一致，默认 agent"
              >
                <Input />
              </Form.Item>
            )}
            <Form.Item name="note" label="备注">
              <Input.TextArea rows={2} />
            </Form.Item>
          </Form>
        )}
      </Modal>

      <Modal
        title="Token 已签发（仅此一次）"
        open={!!issued}
        onCancel={() => setIssued(null)}
        footer={[
          <Button key="close" onClick={() => setIssued(null)}>
            关闭
          </Button>,
        ]}
      >
        {issued ? (
          <Space direction="vertical" style={{ width: "100%" }} size="middle">
            <Alert type="warning" message="请立即复制，之后无法再查看明文。" />
            <Card size="small" title="Bearer token">
              <Typography.Paragraph copyable={{ text: issuedToken }} code>
                {issuedToken}
              </Typography.Paragraph>
              <Button icon={<CopyOutlined />} onClick={() => void copyLabel(issuedToken, "Token")}>
                复制 token
              </Button>
            </Card>
            {issued.kind === "login" ? (
              <Typography.Text type="secondary">
                过期时间 {formatMs(issued.expiresAtMs)}。请求头 Authorization: Bearer。
              </Typography.Text>
            ) : null}
            {issued.kind === "api" ? (
              <Typography.Text type="secondary">
                项目 #{projId}，model 填 {issued.modelAlias}。用于 /v1/responses 与 /v1/chat/completions。
              </Typography.Text>
            ) : null}
            {mcpConfigJson ? (
              <Card size="small" title="mcpServers JSON">
                <Typography.Paragraph>
                  <pre style={{ whiteSpace: "pre-wrap", margin: 0 }}>{mcpConfigJson}</pre>
                </Typography.Paragraph>
                <Button
                  icon={<CopyOutlined />}
                  onClick={() => void copyLabel(mcpConfigJson, "MCP JSON")}
                >
                  复制 mcp.json 片段
                </Button>
              </Card>
            ) : null}
          </Space>
        ) : null}
      </Modal>
    </div>
  );
}
