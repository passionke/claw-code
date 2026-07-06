import {
  AppstoreOutlined,
  ApiOutlined,
  CodeOutlined,
  CommentOutlined,
  FileTextOutlined,
  GlobalOutlined,
  LogoutOutlined,
  PlusOutlined,
  SettingOutlined,
  ToolOutlined,
  UserOutlined,
  FormOutlined,
} from "@ant-design/icons";
import type { MenuProps } from "antd";
import { Avatar, Button, Dropdown, Layout, Menu, Select, Space, Tag, Typography } from "antd";
import { useEffect, useState } from "react";
import { Outlet, useLocation, useNavigate } from "react-router-dom";
import { adminLogout, fetchAdminMe, proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import { isOvsWorkerRelaxed, ovsIdeHref } from "../utils/ovsUrl";

const { Header, Sider, Content } = Layout;

const GLOBAL_MENU_CHILDREN = [
  { key: "/global/inference", label: "全局推理" },
  { key: "/global/e2b-core", label: "核心组件" },
  { key: "/global/strict-landlock", label: "Strict Landlock" },
  { key: "/global/pats", label: "PAT 配置" },
  { key: "/global/admin-mcp", label: "Admin MCP Token" },
  { key: "/global/pools", label: "Pool 集群" },
];

const TAB_ITEMS: MenuProps["items"] = [
  { key: "/", icon: <AppstoreOutlined />, label: "项目" },
  { key: "/skills", icon: <SettingOutlined />, label: "Skills" },
  { key: "/mcp", icon: <ApiOutlined />, label: "MCP" },
  { key: "/claude", icon: <FileTextOutlined />, label: "CLAUDE.md" },
  { key: "/rules", icon: <FileTextOutlined />, label: "Rules" },
  { key: "/preflight", icon: <SettingOutlined />, label: "Preflight" },
  { key: "/worker-profile", icon: <SettingOutlined />, label: "Worker profile" },
  { key: "/prompt", icon: <FileTextOutlined />, label: "系统提示词" },
  { key: "/tools", icon: <ToolOutlined />, label: "Tools" },
  { key: "/extra-session", icon: <FormOutlined />, label: "extraSession" },
  {
    key: "global",
    icon: <GlobalOutlined />,
    label: "全局配置",
    children: GLOBAL_MENU_CHILDREN,
  },
];

export default function AdminLayout() {
  const {
    gatewayBase,
    setGatewayBase,
    gatewayOptions,
    showGatewayPicker,
    projId,
    setProjId,
    projects,
    refreshProjects,
    gatewayImageTag,
    projectConfig,
  } = useApp();
  const loc = useLocation();
  const nav = useNavigate();
  const [adminUser, setAdminUser] = useState("");
  const [openKeys, setOpenKeys] = useState<string[]>([]);
  const selectedKey = (() => {
    if (loc.pathname.startsWith("/global")) {
      return (
        GLOBAL_MENU_CHILDREN.find((c) => loc.pathname.startsWith(c.key))?.key ??
        "/global/inference"
      );
    }
    for (const t of TAB_ITEMS ?? []) {
      if (!t || typeof t !== "object" || !("key" in t)) continue;
      const k = String(t.key);
      if (k === "/" || k === "global") continue;
      if (loc.pathname.startsWith(k)) return k;
    }
    return "/";
  })();

  useEffect(() => {
    if (loc.pathname.startsWith("/global")) {
      setOpenKeys((prev) => (prev.includes("global") ? prev : [...prev, "global"]));
    }
  }, [loc.pathname]);

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
      label: "对话",
    },
    { type: "divider" },
    {
      key: "logout",
      icon: <LogoutOutlined />,
      label: "退出登录",
      danger: true,
    },
  ];

  const projOptions = projects.map((p) => ({
    value: p.projId,
    label:
      p.projectConfigRegistered === false
        ? `项目 ${p.projId} — 未注册`
        : `项目 ${p.projId} — ${p.environmentPrepared ? "就绪" : "未就绪"}`,
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
        {showGatewayPicker ? (
          <>
            <Typography.Text type="secondary">Pool</Typography.Text>
            <Select
              style={{ minWidth: 300 }}
              value={gatewayBase || undefined}
              options={gatewayOptions}
              onChange={setGatewayBase}
            />
          </>
        ) : null}
        {gatewayImageTag ? (
          <Tag color={gatewayImageTag === "local" ? "blue" : "gold"} title="GET /healthz deployImageTag">
            {gatewayImageTag}
          </Tag>
        ) : null}
        <Typography.Text type="secondary">项目</Typography.Text>
        <Select
          style={{ minWidth: 160 }}
          value={projId}
          options={projOptions.length ? projOptions : [{ value: 1, label: "项目 1" }]}
          onChange={setProjId}
        />
        {isOvsWorkerRelaxed(projectConfig?.workerProfileJson) ? (
          <Button href={ovsIdeHref(projId)} target="_blank" rel="noreferrer" icon={<CodeOutlined />}>
            Web IDE
          </Button>
        ) : null}
        <div style={{ flex: 1 }} />
        <Space>
          <Button
            type="primary"
            icon={<PlusOutlined />}
            onClick={async () => {
              const raw = window.prompt("项目 ID（留空自动分配）", "");
              const body: { projId?: number } = {};
              if (raw != null && raw.trim() !== "") {
                const n = parseInt(raw.trim(), 10);
                if (!Number.isFinite(n) || n < 1) return;
                body.projId = n;
              }
              const r = await proxyHttp<{ projId: number }>(
                gatewayBase,
                "POST",
                "/v1/projects",
                body
              );
              await refreshProjects();
              setProjId(r.projId);
            }}
          >
            新建项目
          </Button>
          <Dropdown
            menu={{
              items: userMenuItems,
              onClick: async ({ key }) => {
                if (key === "chat") {
                  nav("/chat");
                  return;
                }
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
            triggerSubMenuAction="click"
            selectedKeys={[selectedKey]}
            openKeys={openKeys}
            onOpenChange={(keys) => setOpenKeys(keys)}
            items={TAB_ITEMS}
            onClick={({ key }) => {
              if (key === "global") return;
              nav(key);
            }}
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
