import { DeleteOutlined, ReloadOutlined } from "@ant-design/icons";
import {
  Alert,
  Button,
  Popconfirm,
  Space,
  Table,
  Tag,
  Tooltip,
  Typography,
  message,
} from "antd";
import type { ColumnsType } from "antd/es/table";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type { ClawPoolEntry } from "../../types/pools";

function formatMs(ms?: number): string {
  if (!ms) return "—";
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

function EllipsisCell({
  text,
  maxWidth,
}: {
  text: string;
  maxWidth: number;
}) {
  return (
    <Tooltip title={text}>
      <Typography.Text
        copyable={text ? { text } : undefined}
        ellipsis
        style={{ maxWidth, display: "inline-block" }}
      >
        {text || "—"}
      </Typography.Text>
    </Tooltip>
  );
}

/** Pool cluster registry from shared PostgreSQL. Author: kejiqing */
export default function GlobalPoolsPage() {
  const { gatewayBase, clusterPools, refreshClusterPools } = useApp();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [deletingId, setDeletingId] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      await refreshClusterPools();
    } catch (e) {
      setError(String((e as Error).message || e));
    } finally {
      setLoading(false);
    }
  }, [refreshClusterPools]);

  useEffect(() => {
    void load();
    const id = window.setInterval(() => void load(), 30_000);
    return () => window.clearInterval(id);
  }, [load]);

  const deletePool = useCallback(
    async (poolId: string) => {
      if (!gatewayBase) return;
      setDeletingId(poolId);
      try {
        await proxyHttp(gatewayBase, "DELETE", `/v1/pools/${encodeURIComponent(poolId)}`);
        message.success(`已删除 ${poolId}`);
        await refreshClusterPools();
      } catch (e) {
        message.error(String((e as Error).message || e));
      } finally {
        setDeletingId(null);
      }
    },
    [gatewayBase, refreshClusterPools]
  );

  const columns: ColumnsType<ClawPoolEntry> = [
    {
      title: "poolId",
      dataIndex: "poolId",
      key: "poolId",
      width: 280,
      fixed: "left",
      render: (v: string) => (
        <EllipsisCell text={v} maxWidth={260} />
      ),
    },
    {
      title: "状态",
      dataIndex: "online",
      key: "online",
      width: 72,
      render: (online: boolean) => (
        <Tag color={online ? "success" : "default"} style={{ margin: 0 }}>
          {online ? "online" : "offline"}
        </Tag>
      ),
    },
    {
      title: "gateway",
      key: "gateway",
      width: 200,
      render: (_, row) => (
        <EllipsisCell text={row.gatewayBase || ""} maxWidth={180} />
      ),
    },
    {
      title: "pool HTTP",
      key: "advertise",
      width: 168,
      render: (_, row) => (
        <EllipsisCell text={`${row.advertiseIp}:${row.ssePort}`} maxWidth={150} />
      ),
    },
    {
      title: "槽位",
      key: "slots",
      width: 64,
      render: (_, row) => `${row.slotsMin}–${row.slotsMax}`,
    },
    {
      title: "心跳",
      dataIndex: "lastHeartbeatMs",
      key: "lastHeartbeatMs",
      width: 158,
      render: (ms: number) => (
        <Typography.Text style={{ fontSize: 12, whiteSpace: "nowrap" }}>
          {formatMs(ms)}
        </Typography.Text>
      ),
    },
    {
      title: "注册",
      dataIndex: "registrationTimeMs",
      key: "registrationTimeMs",
      width: 158,
      render: (ms: number) => (
        <Typography.Text style={{ fontSize: 12, whiteSpace: "nowrap" }}>
          {formatMs(ms)}
        </Typography.Text>
      ),
    },
    {
      title: "操作",
      key: "actions",
      width: 72,
      fixed: "right",
      render: (_, row) => (
        <Popconfirm
          title={
            row.online
              ? "该 pool 仍在线；删除后需 pool-up --restart 才会重新注册。确认？"
              : "删除 PG 中的注册行？pool-daemon 下次启动会重新注册。"
          }
          okText="删除"
          cancelText="取消"
          okButtonProps={{ danger: true }}
          onConfirm={() => void deletePool(row.poolId)}
        >
          <Button
            type="text"
            size="small"
            danger
            icon={<DeleteOutlined />}
            loading={deletingId === row.poolId}
            aria-label={`删除 ${row.poolId}`}
          />
        </Popconfirm>
      ),
    },
  ];

  const offlineCount = (clusterPools?.pools ?? []).filter((p) => !p.online).length;

  return (
    <div style={{ padding: 24, width: "100%", boxSizing: "border-box" }}>
      <Space direction="vertical" size="middle" style={{ width: "100%" }}>
        <Space wrap>
          <Typography.Title level={4} style={{ margin: 0 }}>
            Pool 集群
          </Typography.Title>
          <Button icon={<ReloadOutlined />} loading={loading} onClick={() => void load()}>
            刷新
          </Button>
        </Space>
        {clusterPools?.coLocatedPoolId ? (
          <Typography.Text type="secondary">
            本 Gateway 同机 pool：<Typography.Text code>{clusterPools.coLocatedPoolId}</Typography.Text>
          </Typography.Text>
        ) : null}
        {offlineCount > 0 ? (
          <Typography.Text type="secondary">
            offline 行可删除（僵尸 poolId）；daemon 下次 pool-up 会重新写入。
          </Typography.Text>
        ) : null}
        {error ? <Alert type="error" showIcon message={error} /> : null}
        <Table<ClawPoolEntry>
          rowKey="poolId"
          size="small"
          loading={loading}
          columns={columns}
          dataSource={clusterPools?.pools ?? []}
          pagination={false}
          scroll={{ x: 1080 }}
          tableLayout="fixed"
          locale={{ emptyText: "claw_pool 表暂无注册（检查 pool-daemon 与 PG）" }}
        />
      </Space>
    </div>
  );
}
