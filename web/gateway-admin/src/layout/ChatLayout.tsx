import { AppstoreOutlined } from "@ant-design/icons";
import { Button, Layout, Select, Space, Typography } from "antd";
import { Link, Outlet } from "react-router-dom";
import { useApp } from "../context/AppContext";

const { Header, Content } = Layout;

/** solve_async 对话壳。Author: kejiqing */
export default function ChatLayout() {
  const { gatewayBase, setGatewayBase, gatewayOptions, dsId, setDsId, projects } = useApp();

  const dsOptions = projects.map((p) => ({
    value: p.dsId,
    label: `ds ${p.dsId} — ${p.environmentPrepared ? "就绪" : "未就绪"}`,
  }));

  return (
    <Layout style={{ minHeight: "100vh", background: "#0f1419" }}>
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
        <Space direction="vertical" size={4}>
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            网关
          </Typography.Text>
          <Select
            style={{ minWidth: 200 }}
            value={gatewayBase || undefined}
            options={gatewayOptions}
            onChange={setGatewayBase}
          />
        </Space>
        <Space direction="vertical" size={4}>
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            ds_id
          </Typography.Text>
          <Select
            style={{ minWidth: 160 }}
            value={dsId}
            options={dsOptions.length ? dsOptions : [{ value: 1, label: "ds 1" }]}
            onChange={setDsId}
          />
        </Space>
        <div style={{ flex: 1 }} />
        <Link to="/">
          <Button type="link" icon={<AppstoreOutlined />}>
            项目管理
          </Button>
        </Link>
      </Header>
      <Content style={{ display: "flex", flexDirection: "column", minHeight: 0, flex: 1 }}>
        <Outlet />
      </Content>
    </Layout>
  );
}
