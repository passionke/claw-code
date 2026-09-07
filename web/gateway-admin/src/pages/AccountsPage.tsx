import { PlusOutlined, TeamOutlined } from "@ant-design/icons";
import {
  Button,
  Form,
  Input,
  Modal,
  Select,
  Space,
  Table,
  Tag,
  Typography,
  message,
} from "antd";
import type { ColumnsType } from "antd/es/table";
import { useCallback, useEffect, useState } from "react";
import {
  createAdminAccount,
  deleteAccountProject,
  listAdminAccounts,
  patchAdminAccount,
  putAccountProject,
  type AdminAccountRow,
} from "../api/adminAccounts";
import { useApp } from "../context/AppContext";
import { formatProjectLabel } from "../utils/projectLabel";

function labelForProj(
  projects: { projId: number; projectCode?: string; projectRole?: string; environmentPrepared?: boolean }[],
  id: number
): string {
  const p = projects.find((x) => x.projId === id);
  if (p) return formatProjectLabel(p as Parameters<typeof formatProjectLabel>[0]);
  return `#${id}`;
}

/** System admin: manage accounts and space membership. Author: kejiqing */
export default function AccountsPage() {
  const { gatewayBase, projects } = useApp();
  const [rows, setRows] = useState<AdminAccountRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [memberOpen, setMemberOpen] = useState<AdminAccountRow | null>(null);
  const [form] = Form.useForm();
  const [memberForm] = Form.useForm();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setRows(await listAdminAccounts(gatewayBase));
    } catch (e) {
      message.error(e instanceof Error ? e.message : "加载账号失败");
      setRows([]);
    } finally {
      setLoading(false);
    }
  }, [gatewayBase]);

  useEffect(() => {
    void load();
  }, [load]);

  const columns: ColumnsType<AdminAccountRow> = [
    { title: "用户名", dataIndex: "username", width: 140 },
    {
      title: "角色",
      dataIndex: "systemRole",
      width: 120,
      render: (v: string) =>
        v === "system_admin" ? (
          <Tag color="blue">system_admin</Tag>
        ) : (
          <Tag>space</Tag>
        ),
    },
    {
      title: "空间 (projId)",
      dataIndex: "projectIds",
      render: (ids: number[]) =>
        (ids || []).length
          ? ids.map((id) => (
              <Tag key={id}>{labelForProj(projects, id)}</Tag>
            ))
          : "—",
    },
    {
      title: "状态",
      dataIndex: "disabled",
      width: 90,
      render: (d: boolean) => (d ? <Tag color="red">禁用</Tag> : <Tag color="green">启用</Tag>),
    },
    {
      title: "操作",
      width: 280,
      render: (_, row) => (
        <Space wrap>
          <Button size="small" onClick={() => setMemberOpen(row)}>
            空间成员
          </Button>
          <Button
            size="small"
            onClick={async () => {
              await patchAdminAccount(gatewayBase, row.accountId, {
                disabled: !row.disabled,
              });
              message.success(row.disabled ? "已启用" : "已禁用");
              await load();
            }}
          >
            {row.disabled ? "启用" : "禁用"}
          </Button>
          <Button
            size="small"
            onClick={async () => {
              const next =
                row.systemRole === "system_admin" ? "none" : "system_admin";
              await patchAdminAccount(gatewayBase, row.accountId, {
                systemRole: next,
              });
              message.success("角色已更新");
              await load();
            }}
          >
            {row.systemRole === "system_admin" ? "降为空间账号" : "升为 admin"}
          </Button>
        </Space>
      ),
    },
  ];

  return (
    <div style={{ padding: 16 }}>
      <Space style={{ marginBottom: 16 }} wrap>
        <Typography.Title level={4} style={{ margin: 0 }}>
          <TeamOutlined /> 账号管理
        </Typography.Title>
        <Button type="primary" icon={<PlusOutlined />} onClick={() => setCreateOpen(true)}>
          创建账号
        </Button>
        <Button onClick={() => void load()}>刷新</Button>
      </Space>
      <Table
        rowKey="accountId"
        loading={loading}
        columns={columns}
        dataSource={rows}
        pagination={false}
        size="middle"
      />

      <Modal
        title="创建账号"
        open={createOpen}
        onCancel={() => setCreateOpen(false)}
        onOk={async () => {
          const v = await form.validateFields();
          await createAdminAccount(gatewayBase, {
            username: v.username.trim(),
            password: v.password,
            systemRole: v.systemRole || "none",
          });
          message.success("已创建");
          setCreateOpen(false);
          form.resetFields();
          await load();
        }}
        destroyOnClose
      >
        <Form form={form} layout="vertical" initialValues={{ systemRole: "none" }}>
          <Form.Item name="username" label="用户名" rules={[{ required: true }]}>
            <Input autoComplete="off" />
          </Form.Item>
          <Form.Item
            name="password"
            label="密码"
            rules={[{ required: true, min: 6 }]}
          >
            <Input.Password />
          </Form.Item>
          <Form.Item name="systemRole" label="系统角色">
            <Select
              options={[
                { value: "none", label: "普通（靠空间成员成为空间管理员）" },
                { value: "system_admin", label: "system_admin" },
              ]}
            />
          </Form.Item>
        </Form>
      </Modal>

      <Modal
        title={memberOpen ? `空间成员 — ${memberOpen.username}` : "空间成员"}
        open={!!memberOpen}
        onCancel={() => setMemberOpen(null)}
        footer={null}
        destroyOnClose
      >
        {memberOpen && (
          <>
            <Form
              form={memberForm}
              layout="inline"
              style={{ marginBottom: 12 }}
              onFinish={async (v) => {
                await putAccountProject(
                  gatewayBase,
                  memberOpen.accountId,
                  Number(v.projId)
                );
                message.success("已加入空间");
                memberForm.resetFields();
                await load();
                const next = await listAdminAccounts(gatewayBase);
                setMemberOpen(
                  next.find((a) => a.accountId === memberOpen.accountId) || null
                );
              }}
            >
              <Form.Item name="projId" rules={[{ required: true }]}>
                <Select
                  style={{ width: 260 }}
                  placeholder="选择项目"
                  options={projects.map((p) => ({
                    value: p.projId,
                    label: formatProjectLabel(p),
                  }))}
                />
              </Form.Item>
              <Button type="primary" htmlType="submit">
                加入
              </Button>
            </Form>
            <Space wrap>
              {(memberOpen.projectIds || []).map((id) => (
                <Tag
                  key={id}
                  closable
                  onClose={async (e) => {
                    e.preventDefault();
                    await deleteAccountProject(
                      gatewayBase,
                      memberOpen.accountId,
                      id
                    );
                    message.success("已移除");
                    const next = await listAdminAccounts(gatewayBase);
                    setRows(next);
                    setMemberOpen(
                      next.find((a) => a.accountId === memberOpen.accountId) ||
                        null
                    );
                  }}
                >
                  {formatProjectLabel(
                    projects.find((p) => p.projId === id) ||
                      ({
                        projId: id,
                        environmentPrepared: false,
                      } as Parameters<typeof formatProjectLabel>[0])
                  )}
                </Tag>
              ))}
            </Space>
            <div style={{ marginTop: 16 }}>
              <Typography.Text type="secondary">
                改密：在下方输入新密码后按回车保存
              </Typography.Text>
              <Input.Password
                style={{ marginTop: 8, maxWidth: 280 }}
                placeholder="新密码（至少 6 位）"
                onPressEnter={async (e) => {
                  const pw = (e.target as HTMLInputElement).value;
                  if (pw.length < 6) {
                    message.error("密码至少 6 位");
                    return;
                  }
                  await patchAdminAccount(gatewayBase, memberOpen.accountId, {
                    password: pw,
                  });
                  message.success("密码已更新");
                }}
              />
            </div>
          </>
        )}
      </Modal>
    </div>
  );
}
