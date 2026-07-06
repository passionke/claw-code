import { AppstoreOutlined, CodeOutlined } from "@ant-design/icons";
import { Button, Layout, Select, Space, Typography } from "antd";
import { Link, Outlet } from "react-router-dom";
import { useApp } from "../context/AppContext";
import { isOvsWorkerRelaxed, ovsIdeHref } from "../utils/ovsUrl";

const { Header, Content } = Layout;

/** solve_async 对话壳。Author: kejiqing */
export default function ChatLayout() {
  const {
    gatewayBase,
    setGatewayBase,
    gatewayOptions,
    showGatewayPicker,
    projId,
    setProjId,
    projects,
    projectConfig,
  } = useApp();

  const projOptions = projects.map((p) => ({
    value: p.projId,
    label:
      p.projectConfigRegistered === false
        ? `项目 ${p.projId} — 未注册`
        : `项目 ${p.projId} — ${p.environmentPrepared ? "就绪" : "未就绪"}`,
  }));

  return (
    <Layout
      style={{
        height: "100vh",
        display: "flex",
        flexDirection: "column",
        background: "#0f1419",
        overflow: "hidden",
      }}
    >
      <Header
        style={{
          display: "flex",
          flexWrap: "wrap",
          alignItems: "flex-end",
          gap: 10,
          padding: "12px 16px",
          background: "#1a2332",
          height: "auto",
          lineHeight: 1.4,
        }}
      >
        {showGatewayPicker ? (
          <Space direction="vertical" size={4}>
            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
              Pool
            </Typography.Text>
            <Select
              style={{ minWidth: 280 }}
              value={gatewayBase || undefined}
              options={gatewayOptions}
              onChange={setGatewayBase}
            />
          </Space>
        ) : null}
        <Space direction="vertical" size={4}>
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            项目
          </Typography.Text>
          <Select
            style={{ minWidth: 160 }}
            value={projId}
            options={projOptions.length ? projOptions : [{ value: 1, label: "项目 1" }]}
            onChange={setProjId}
          />
        </Space>
        {isOvsWorkerRelaxed(projectConfig?.workerProfileJson) ? (
          <Button href={ovsIdeHref(projId)} target="_blank" rel="noreferrer" icon={<CodeOutlined />}>
            Web IDE
          </Button>
        ) : null}
        <div style={{ flex: 1 }} />
        <Link to="/">
          <Button type="link" icon={<AppstoreOutlined />}>
            项目管理
          </Button>
        </Link>
      </Header>
      <Content
        style={{
          display: "flex",
          flexDirection: "column",
          flex: 1,
          minHeight: 0,
          overflow: "hidden",
        }}
      >
        <Outlet />
      </Content>
    </Layout>
  );
}
