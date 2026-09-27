import { AppstoreOutlined } from "@ant-design/icons";
import { Button, Layout, Select, Typography } from "antd";
import { Link, Outlet } from "react-router-dom";
import { useApp } from "../context/AppContext";
import { formatProjectLabel } from "../utils/projectLabel";

const { Header, Content } = Layout;

/** Playground 主聊天壳（POST /v1/responses 真流）。Author: kejiqing */
export default function ChatLayout() {
  const {
    projId,
    setProjId,
    projects,
  } = useApp();

  const projOptions = projects.map((p) => ({
    value: p.projId,
    label: formatProjectLabel(p),
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
          alignItems: "center",
          gap: 12,
          padding: "0 16px",
          background: "#1a2332",
          height: "auto",
          minHeight: 64,
          lineHeight: 1.4,
        }}
      >
        <Typography.Text type="secondary">项目</Typography.Text>
        <Select
          style={{ minWidth: 280 }}
          value={projId}
          options={projOptions.length ? projOptions : [{ value: 1, label: "#1" }]}
          onChange={setProjId}
        />
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
