import {
  AppstoreOutlined,
  ApiOutlined,
  CommentOutlined,
  FileTextOutlined,
  GlobalOutlined,
  LogoutOutlined,
  PlusOutlined,
  SettingOutlined,
  ToolOutlined,
  UserOutlined,
} from "@ant-design/icons";
import type { MenuProps } from "antd";
import { Avatar, Button, Dropdown, Layout, Menu, Select, Space, Tag, Typography } from "antd";
import { useEffect, useState } from "react";
import { Outlet, useLocation, useNavigate } from "react-router-dom";
import { adminLogout, fetchAdminMe, proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";

const { Header, Sider, Content } = Layout;

const TAB_ITEMS = [
  { key: "/", icon: <AppstoreOutlined />, label: "项目" },
  { key: "/skills", icon: <SettingOutlined />, label: "Skills" },
  { key: "/mcp", icon: <ApiOutlined />, label: "MCP" },
  { key: "/claude", icon: <FileTextOutlined />, label: "CLAUDE.md" },
  { key: "/rules", icon: <FileTextOutlined />, label: "Rules" },
  { key: "/prompt", icon: <FileTextOutlined />, label: "系统提示词" },
  { key: "/tools", icon: <ToolOutlined />, label: "Tools" },
  { key: "/global", icon: <GlobalOutlined />, label: "全局配置" },
];

export default function AdminLayout() {
  const {
    gatewayBase,
    setGatewayBase,
    gatewayOptions,
    dsId,
    setDsId,
    projects,
    refreshProjects,
    gatewayImageTag,
  } = useApp();
  const loc = useLocation();
  const nav = useNavigate();
  const [adminUser, setAdminUser] = useState("");
  const selectedKey =
    TAB_ITEMS.find((t) => t.key !== "/" && loc.pathname.startsWith(t.key))?.key ||
    "/";

  useEffect(() => {
    fetchAdminMe()
      .then((r) => {
        if (r.ok && r.user) setAdminUser(r.user);
      })
      .catch(() => setAdminUser(""));
  }, []);

  useEffect(() => {
    if (selectedKey === "/") {
      const t = window.setInterval(() => refreshProjects(true), 15000);
      return () => clearInterval(t);
    }
  }, [selectedKey, refreshProjects]);

  const userMenuItems: MenuProps["items"] = [
    {
      key: "chat",
      icon: <CommentOutlined />,
      label: (
        <a href="/" style={{ color: "inherit" }}>
          对话
        </a>
      ),
    },
    { type: "divider" },
    {
      key: "logout",
      icon: <LogoutOutlined />,
      label: "退出登录",
      danger: true,
    },
  ];

  const dsOptions = projects.map((p) => ({
    value: p.dsId,
    label: `ds ${p.dsId} — ${p.environmentPrepared ? "就绪" : "未就绪"}`,
  }));

  return (
    <Layout style={{ minHeight: "100vh" }}>
      <Header
        style={{
          display: "flex",
          alignItems: "center",
          gap: 12,
          padding: "0 16px",
          background: "#1a2332",
        }}
      >
        <Typography.Text type="secondary">网关</Typography.Text>
        <Select
          style={{ minWidth: 220 }}
          value={gatewayBase || undefined}
          options={gatewayOptions}
          onChange={setGatewayBase}
        />
        {gatewayImageTag ? (
          <Tag color={gatewayImageTag === "local" ? "blue" : "gold"} title="GET /healthz deployImageTag">
            {gatewayImageTag}
          </Tag>
        ) : null}
        <Typography.Text type="secondary">ds_id</Typography.Text>
        <Select
          style={{ minWidth: 160 }}
          value={dsId}
          options={dsOptions.length ? dsOptions : [{ value: 1, label: "ds 1" }]}
          onChange={setDsId}
        />
        <div style={{ flex: 1 }} />
        <Space>
          <Button
            type="primary"
            icon={<PlusOutlined />}
            onClick={async () => {
              const raw = window.prompt("ds_id（留空自动分配）", "");
              const body: { dsId?: number } = {};
              if (raw != null && raw.trim() !== "") {
                const n = parseInt(raw.trim(), 10);
                if (!Number.isFinite(n) || n < 1) return;
                body.dsId = n;
              }
              const r = await proxyHttp<{ dsId: number }>(
                gatewayBase,
                "POST",
                "/v1/projects",
                body
              );
              await refreshProjects();
              setDsId(r.dsId);
            }}
          >
            新建项目
          </Button>
          <Dropdown
            menu={{
              items: userMenuItems,
              onClick: async ({ key }) => {
                if (key === "logout") {
                  await adminLogout();
                  nav("/login");
                }
              },
            }}
            placement="bottomRight"
            trigger={["click"]}
          >
            <Space style={{ cursor: "pointer", padding: "4px 8px" }}>
              <Avatar size="small" icon={<UserOutlined />} style={{ background: "#2563eb" }} />
              <Typography.Text>{adminUser || "管理员"}</Typography.Text>
            </Space>
          </Dropdown>
        </Space>
      </Header>
      <Layout>
        <Sider width={200} style={{ background: "#1a2332" }}>
          <Menu
            mode="inline"
            selectedKeys={[selectedKey]}
            items={TAB_ITEMS}
            onClick={({ key }) => nav(key)}
            style={{ height: "100%", borderRight: 0 }}
          />
        </Sider>
        <Content style={{ padding: 16, overflow: "auto" }}>
          <Outlet />
        </Content>
      </Layout>
    </Layout>
  );
}
