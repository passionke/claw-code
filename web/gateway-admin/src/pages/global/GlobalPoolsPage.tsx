import { ReloadOutlined } from "@ant-design/icons";
import { Alert, Button, Space, Table, Tag, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type { ClawPoolEntry, ListClawPoolsResponse } from "../../types/pools";

function formatMs(ms?: number): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

/** Pool cluster registry from shared PostgreSQL. Author: kejiqing */
export default function GlobalPoolsPage() {
  const { gatewayBase } = useApp();
  const [loading, setLoading] = useState(false);
  const [data, setData] = useState<ListClawPoolsResponse | null>(null);
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    if (!gatewayBase) return;
    setLoading(true);
    setError("");
    try {
      const r = await proxyHttp<ListClawPoolsResponse>(
        gatewayBase,
        "GET",
        "/v1/pools"
      );
      setData(r);
    } catch (e) {
      setError(String((e as Error).message || e));
      setData(null);
    } finally {
      setLoading(false);
    }
  }, [gatewayBase]);

  useEffect(() => {
    void load();
    const id = window.setInterval(() => void load(), 30_000);
    return () => window.clearInterval(id);
  }, [load]);

  const columns: ColumnsType<ClawPoolEntry> = [
    {
      title: "poolId",
      dataIndex: "poolId",
      key: "poolId",
      render: (v: string) => <Typography.Text code>{v}</Typography.Text>,
    },
    {
      title: "状态",
      dataIndex: "online",
      key: "online",
      width: 90,
      render: (online: boolean) => (
        <Tag color={online ? "success" : "default"}>
          {online ? "online" : "offline"}
        </Tag>
      ),
    },
    {
      title: "advertise",
      key: "advertise",
      render: (_, row) => (
        <Typography.Text copyable={{ text: row.httpBase }}>
          {row.advertiseIp}:{row.ssePort}
        </Typography.Text>
      ),
    },
    {
      title: "槽位",
      key: "slots",
      width: 100,
      render: (_, row) => `${row.slotsMin}–${row.slotsMax}`,
    },
    {
      title: "心跳",
      dataIndex: "lastHeartbeatMs",
      key: "lastHeartbeatMs",
      render: (ms: number) => formatMs(ms),
    },
    {
      title: "注册",
      dataIndex: "registrationTimeMs",
      key: "registrationTimeMs",
      render: (ms: number) => formatMs(ms),
    },
  ];

  return (
    <div style={{ padding: 24, maxWidth: 1100 }}>
      <Space direction="vertical" size="middle" style={{ width: "100%" }}>
        <Space wrap>
          <Typography.Title level={4} style={{ margin: 0 }}>
            Pool 集群
          </Typography.Title>
          <Button icon={<ReloadOutlined />} loading={loading} onClick={() => void load()}>
            刷新
          </Button>
        </Space>
        {data?.coLocatedPoolId ? (
          <Typography.Text type="secondary">
            本 Gateway 同机 pool：<Typography.Text code>{data.coLocatedPoolId}</Typography.Text>
          </Typography.Text>
        ) : null}
        {error ? <Alert type="error" showIcon message={error} /> : null}
        <Table<ClawPoolEntry>
          rowKey="poolId"
          size="small"
          loading={loading}
          columns={columns}
          dataSource={data?.pools ?? []}
          pagination={false}
          locale={{ emptyText: "claw_pool 表暂无注册（检查 pool-daemon 与 PG）" }}
        />
      </Space>
    </div>
  );
}
