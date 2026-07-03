import { EyeOutlined, LinkOutlined, ReloadOutlined, SyncOutlined } from "@ant-design/icons";
import {
  Alert,
  Button,
  Card,
  Descriptions,
  Form,
  Input,
  Popconfirm,
  Space,
  Tag,
  Typography,
  message,
} from "antd";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type {
  ClawTapSettings,
  GlobalSettingsResponse,
  ObserveTapResetResponse,
} from "../../types/globalSettings";
import LlmModelsPage from "./LlmModelsPage";

function formatMs(ms?: number): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

function formatRemainingTtl(secs?: number): string {
  if (secs === undefined || secs === null) return "—";
  if (secs <= 0) return "已过期";
  const days = Math.floor(secs / 86_400);
  const hours = Math.floor((secs % 86_400) / 3600);
  const minutes = Math.floor((secs % 3600) / 60);
  if (days > 0) return `${days} 天 ${hours} 小时`;
  if (hours > 0) return `${hours} 小时 ${minutes} 分钟`;
  return `${minutes} 分钟`;
}

function observeRuntimeTag(tap: ClawTapSettings | null): ReactNode {
  if (!tap?.e2bObserveSandboxId) return null;
  if (tap.e2bObserveSandboxRunning === true) {
    const remaining = tap.e2bObserveSandboxRemainingTtlSecs;
    if (remaining !== undefined && remaining <= 3_600) {
      return <Tag color="warning">即将过期</Tag>;
    }
    return <Tag color="success">running</Tag>;
  }
  if (tap.e2bObserveSandboxRunning === false) {
    return (
      <Tag color="error">
        {tap.e2bObserveSandboxState ? tap.e2bObserveSandboxState : "已停止"}
      </Tag>
    );
  }
  return null;
}

function observeStatusTag(tap: ClawTapSettings | null): ReactNode {
  if (!tap?.configured) {
    return <Tag>未初始化</Tag>;
  }
  if (tap.e2bObserveSandboxId && tap.proxyBaseUrl) {
    const runtime = observeRuntimeTag(tap);
    if (tap.e2bObserveSandboxRunning === false) {
      return (
        <Space size="small">
          <Tag color="error">PG 已绑定但沙箱不可用</Tag>
          {runtime}
        </Space>
      );
    }
    return (
      <Space size="small">
        <Tag color="success">observe 已绑定</Tag>
        {runtime}
      </Space>
    );
  }
  if (tap.proxyBaseUrl) {
    return <Tag color="warning">代理已配置（缺 sandboxId）</Tag>;
  }
  return <Tag color="warning">等待 observe-tap-up</Tag>;
}

/** e2b 全局推理：集群 ID + LLM 模型列表 + observe 单例只读信息。Author: kejiqing */
export default function GlobalInferencePage() {
  const { gatewayBase } = useApp();
  const [loading, setLoading] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [clusterId, setClusterId] = useState("");
  const [observeTap, setObserveTap] = useState<ClawTapSettings | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      setClusterId(r.clusterId ?? "");
      setObserveTap(r.clawTap ?? null);
    } finally {
      setLoading(false);
    }
  }, [gatewayBase]);

  useEffect(() => {
    load().catch(() => {});
  }, [load]);

  const resetObserve = async () => {
    setResetting(true);
    try {
      const r = await proxyHttp<ObserveTapResetResponse>(
        gatewayBase,
        "POST",
        "/v1/gateway/global-settings/observe-tap/reset"
      );
      setObserveTap(r.tap ?? null);
      if (r.trafficReachable) {
        message.success(`observe 已重置（${r.sandboxId}）`);
      } else {
        message.warning(r.message ?? "observe 已重建，但 traffic 探测未通过");
      }
      await load();
    } catch (e) {
      message.error(String(e));
    } finally {
      setResetting(false);
    }
  };

  const sandboxId = (observeTap?.e2bObserveSandboxId ?? "").trim();
  const proxyUrl = (observeTap?.proxyBaseUrl ?? "").trim();
  const liveUrl = (observeTap?.liveBaseUrl ?? "").trim();

  return (
    <div style={{ maxWidth: 960 }}>
      <Typography.Title level={4} style={{ marginTop: 0 }}>
        全局推理
      </Typography.Title>

      <Alert
        type="info"
        showIcon
        style={{ marginBottom: 16 }}
        message="e2b 模式：clawTap = observe 沙箱单例"
        description={
          <Typography.Paragraph style={{ marginBottom: 0 }}>
            LLM 上游与 API Key 在本页配置；worker solve 时 gateway 将{" "}
            <Typography.Text code>OPENAI_BASE_URL</Typography.Text> 指向 observe 沙箱代理（8080）。
            代理端点与 sandboxId 由{" "}
            <Typography.Text code>gateway.sh observe-tap-up</Typography.Text> 写入 PG，不在此手填
            IP/端口。
          </Typography.Paragraph>
        }
      />

      <Form layout="vertical" style={{ marginBottom: 16 }}>
        <Form.Item label="集群 ID" extra="来自 gateway 进程环境变量 CLAW_CLUSTER_ID，只读">
          <Input
            readOnly
            value={clusterId}
            placeholder={loading ? "" : "未设置（检查 .env 与 gateway.sh up）"}
            style={{ maxWidth: 360, cursor: "default" }}
          />
        </Form.Item>
      </Form>

      <Card
        title={
          <span>
            <EyeOutlined /> observe worker（LLM 代理单例）
          </span>
        }
        loading={loading}
        style={{ marginBottom: 16 }}
        extra={
          <Space>
            <Button icon={<ReloadOutlined />} loading={loading} onClick={() => void load()}>
              刷新
            </Button>
            <Popconfirm
              title="重置 observe 沙箱？"
              description="删除当前 observe sandbox 并重建 claude-tap（约 1–2 分钟）。"
              onConfirm={() => void resetObserve()}
              okText="重置"
              cancelText="取消"
            >
              <Button type="primary" icon={<SyncOutlined />} loading={resetting}>
                重置 observe
              </Button>
            </Popconfirm>
          </Space>
        }
      >
        {!observeTap?.configured ? (
          <Alert
            type="warning"
            showIcon
            style={{ marginBottom: 16 }}
            message="observe 尚未初始化"
            description="运行 ./deploy/stack/gateway.sh observe-tap-up --reuse，或点击「重置 observe」。"
          />
        ) : null}

        <Descriptions column={1} bordered size="small">
          <Descriptions.Item label="状态">{observeStatusTag(observeTap)}</Descriptions.Item>
          <Descriptions.Item label="沙箱运行状态">
            {observeTap?.e2bObserveSandboxState ? (
              <Space size="small">
                <Typography.Text code>{observeTap.e2bObserveSandboxState}</Typography.Text>
                {observeRuntimeTag(observeTap)}
              </Space>
            ) : sandboxId ? (
              <Typography.Text type="secondary">未探测（gateway 无 e2b 客户端）</Typography.Text>
            ) : (
              "—"
            )}
          </Descriptions.Item>
          <Descriptions.Item label="过期时间">
            {formatMs(observeTap?.e2bObserveSandboxEndAtMs)}
          </Descriptions.Item>
          <Descriptions.Item label="剩余 TTL">
            {formatRemainingTtl(observeTap?.e2bObserveSandboxRemainingTtlSecs)}
          </Descriptions.Item>
          <Descriptions.Item label="沙箱 ID">
            {sandboxId ? (
              <Typography.Text code copyable>
                {sandboxId}
              </Typography.Text>
            ) : (
              "—"
            )}
          </Descriptions.Item>
          <Descriptions.Item label="代理端点（worker OPENAI_BASE_URL）">
            {proxyUrl ? (
              <Typography.Paragraph copyable={{ text: proxyUrl }} style={{ marginBottom: 0 }}>
                <Typography.Text code>{proxyUrl}</Typography.Text>
              </Typography.Paragraph>
            ) : (
              "—"
            )}
          </Descriptions.Item>
          <Descriptions.Item label="Live 观测">
            {liveUrl ? (
              <Space direction="vertical" size="small">
                <Button
                  size="small"
                  icon={<LinkOutlined />}
                  href={liveUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  打开 Live
                </Button>
                <Typography.Text copyable style={{ wordBreak: "break-all" }}>
                  {liveUrl}
                </Typography.Text>
              </Space>
            ) : (
              "—"
            )}
          </Descriptions.Item>
          <Descriptions.Item label="PG 更新时间">
            {formatMs(observeTap?.updatedAtMs)}
          </Descriptions.Item>
        </Descriptions>
      </Card>

      <Card title="大模型列表" style={{ marginTop: 0 }}>
        <LlmModelsPage embedded />
      </Card>
    </div>
  );
}
