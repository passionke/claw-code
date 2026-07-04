import { EyeOutlined, LinkOutlined, ReloadOutlined, SyncOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Descriptions, Popconfirm, Space, Tag, Typography, message } from "antd";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type { ClawTapSettings, GlobalSettingsResponse, ObserveTapResetResponse } from "../../types/globalSettings";

export type { ObserveTapResetResponse };

function formatMs(ms?: number): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

function onlineTag(tap: ClawTapSettings | null): ReactNode {
  if (!tap?.configured) {
    return <Tag>未配置</Tag>;
  }
  if (tap.proxyBaseUrl && tap.liveBaseUrl) {
    return <Tag color="success">代理 + Live 已配置</Tag>;
  }
  if (tap.proxyBaseUrl) {
    return <Tag color="processing">代理已配置</Tag>;
  }
  if (tap.liveBaseUrl) {
    return <Tag color="processing">Live 已配置</Tag>;
  }
  return <Tag color="warning">等待初始化</Tag>;
}

/** e2b e2b observe singleton — full-featured Tap proxy + Live viewer. Author: kejiqing */
export default function ObserveTapPage() {
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
    } catch (e) {
      message.error(`加载观测 Tap 失败：${String(e)}`);
    } finally {
      setLoading(false);
    }
  }, [gatewayBase]);

  const resetTap = useCallback(async () => {
    setResetting(true);
    try {
      const r = await proxyHttp<ObserveTapResetResponse>(
        gatewayBase,
        "POST",
        "/v1/gateway/global-settings/observe-tap/reset"
      );
      setObserveTap(r.tap ?? null);
      if (r.trafficReachable) {
        message.success(`Tap 已重置（sandbox ${r.sandboxId}）`);
      } else {
        message.warning(r.message ?? "Tap 已重建，但 traffic 探测未通过");
      }
    } catch (e) {
      message.error(`重置 Tap 失败：${String(e)}`);
    } finally {
      setResetting(false);
    }
  }, [gatewayBase]);

  useEffect(() => {
    void load();
  }, [load]);

  const liveUrl = (observeTap?.liveBaseUrl ?? "").trim();
  const proxyUrl = (observeTap?.proxyBaseUrl ?? "").trim();

  return (
    <Space direction="vertical" size="large" style={{ width: "100%", maxWidth: 960 }}>
      <Space style={{ width: "100%", justifyContent: "space-between" }}>
        <Typography.Title level={4} style={{ margin: 0 }}>
          <EyeOutlined /> 全功能 Tap
        </Typography.Title>
        <Button icon={<ReloadOutlined />} loading={loading} onClick={() => void load()}>
          刷新
        </Button>
      </Space>

      <Alert
        type="info"
        showIcon
        message="FC 集群共享的标准 LLM 代理"
        description={
          <Typography.Paragraph style={{ marginBottom: 0 }}>
            这是 e2b 上的<strong>长周期 Tap 单例</strong>，同时提供 worker 使用的 OpenAI
            兼容代理（8080）和 trace Live 浏览（3000）。
            <br />
            LLM 配置由 tap 通过 PG 自行感知并热更新；Gateway 只把
            <Typography.Text code>OPENAI_BASE_URL</Typography.Text> 指向这里。
            <br />
            Live / 代理地址由 gateway 启动时自动 ensure 写入 PG；「重置 Tap」会先杀旧 observe sandbox 再重建。
          </Typography.Paragraph>
        }
      />

      {!observeTap?.configured ? (
        <Alert
          type="warning"
          showIcon
          message="尚未初始化 Tap"
          description="gateway 启动后会自动 ensure observe 单例，或在下方点击「重置 Tap」。"
        />
      ) : null}

      <Card title="当前状态" loading={loading}>
        <Descriptions column={1} bordered size="small">
          <Descriptions.Item label="在线状态">
            <Space wrap>
              {onlineTag(observeTap)}
              {proxyUrl ? <Tag color="blue">标准代理</Tag> : null}
              {liveUrl ? <Tag color="purple">Live 浏览</Tag> : null}
            </Space>
          </Descriptions.Item>
          <Descriptions.Item label="集群 ID">{clusterId || "—"}</Descriptions.Item>
          <Descriptions.Item label="类型">
            <Tag color="purple">e2b Tap 单例</Tag>
          </Descriptions.Item>
          <Descriptions.Item label="代理入口">
            {proxyUrl ? (
              <Typography.Paragraph copyable={{ text: proxyUrl }} style={{ marginBottom: 0 }}>
                <Typography.Text code>{proxyUrl}</Typography.Text>
              </Typography.Paragraph>
            ) : (
              "—"
            )}
          </Descriptions.Item>
          <Descriptions.Item label="配置更新时间">{formatMs(observeTap?.updatedAtMs)}</Descriptions.Item>
        </Descriptions>

        <div style={{ marginTop: 16 }}>
          <Typography.Text strong>Live 观测地址</Typography.Text>
          <Typography.Paragraph type="secondary" style={{ marginBottom: 8, marginTop: 4 }}>
            点击下方链接在新标签页打开；也可复制地址。
          </Typography.Paragraph>
          {liveUrl ? (
            <Space direction="vertical" size="small" style={{ width: "100%" }}>
              <Button
                type="primary"
                icon={<LinkOutlined />}
                href={liveUrl}
                target="_blank"
                rel="noopener noreferrer"
              >
                打开 Live 观测
              </Button>
              <Typography.Paragraph
                copyable={{ text: liveUrl }}
                style={{ marginBottom: 0, wordBreak: "break-all" }}
              >
                <Typography.Link href={liveUrl} target="_blank" rel="noopener noreferrer">
                  {liveUrl}
                </Typography.Link>
              </Typography.Paragraph>
            </Space>
          ) : (
            <Typography.Text type="secondary">未配置</Typography.Text>
          )}
        </div>

        <Space style={{ marginTop: 16 }}>
          <Popconfirm
            title="重置 Tap 单例？"
            description="将删除当前 observe sandbox 并重建 claude-tap Live（约 1–2 分钟）。"
            onConfirm={() => void resetTap()}
            okText="重置"
            cancelText="取消"
          >
            <Button type="primary" icon={<SyncOutlined />} loading={resetting}>
              重置 Tap
            </Button>
          </Popconfirm>
          <Button icon={<ReloadOutlined />} loading={loading} onClick={() => void load()}>
            刷新
          </Button>
        </Space>
      </Card>
    </Space>
  );
}
