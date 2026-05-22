import { DeleteOutlined, PlusOutlined } from "@ant-design/icons";
import {
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

interface GlobalSettingsResponse {
  updatedAtMs: number;
  gitPats: GitPatRow[];
}

export default function GlobalSettingsPage() {
  const { gatewayBase } = useApp();
  const [pats, setPats] = useState<GitPatRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [editing, setEditing] = useState<GitPatRow | null>(null);
  const [form] = Form.useForm();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      setPats(r.gitPats || []);
    } finally {
      setLoading(false);
    }
  }, [gatewayBase]);

  useEffect(() => {
    load().catch(() => setPats([]));
  }, [load]);

  const openCreate = () => {
    setEditing(null);
    form.resetFields();
    setModalOpen(true);
  };

  const openEdit = (row: GitPatRow) => {
    setEditing(row);
    form.setFieldsValue({ name: row.name, note: row.note || "" });
    setModalOpen(true);
  };

  const savePat = async () => {
    const v = await form.validateFields();
    const body: {
      id?: string;
      name: string;
      note?: string;
      token?: string;
    } = {
      name: (v.name || "").trim(),
      note: (v.note || "").trim() || undefined,
    };
    if (editing) {
      body.id = editing.id;
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
    message.success(editing ? "PAT 已更新" : "PAT 已添加");
    setModalOpen(false);
    await load();
  };

  const columns: ColumnsType<GitPatRow> = [
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
          <Button size="small" onClick={() => openEdit(row)}>
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
            <Button size="small" danger icon={<DeleteOutlined />}>
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
      <Typography.Paragraph type="secondary">
        与 ds_id 无关的网关级设置。Git 单向同步在各项目中通过下拉选择此处登记的 PAT，不在项目配置里保存
        Token 明文。
      </Typography.Paragraph>

      <Card
        title="Git PAT"
        size="small"
        extra={
          <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>
            添加 PAT
          </Button>
        }
      >
        <Table
          rowKey="id"
          size="small"
          loading={loading}
          columns={columns}
          dataSource={pats}
          pagination={false}
        />
      </Card>

      <Modal
        title={editing ? `编辑 PAT · ${editing.id}` : "添加 PAT"}
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={() => savePat().catch((e) => message.error(String(e)))}
        destroyOnClose
      >
        <Form form={form} layout="vertical">
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
            label={editing ? "Token（留空表示不修改）" : "Token"}
            rules={editing ? [] : [{ required: true, message: "请填写 Token" }]}
          >
            <Input.Password placeholder="Personal Access Token" />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
