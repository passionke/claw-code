import { ApiOutlined, ReloadOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Descriptions, Space, Tag, Typography } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import TemplateBuildStep from "../../components/bootstrap/TemplateBuildStep";
import { useApp } from "../../context/AppContext";
import type {
  ClusterBootstrapSnapshot,
  E2bPlatformSettings,
  E2bWorkerSettings,
  GlobalSettingsResponse,
} from "../../types/globalSettings";

/**
 * e2b 平台：只读连接信息 + 与 Init 相同的「制作/升级模板」能力。
 * CLI 升级、gateway 不变时在此重打模板。Author: kejiqing
 */
export default function E2bPlatformPage() {
  const { gatewayBase } = useApp();
  const [loading, setLoading] = useState(false);
  const [settings, setSettings] = useState<E2bPlatformSettings | null>(null);
  const [e2bWorker, setE2bWorker] = useState<E2bWorkerSettings | null>(null);
  const [bootstrapSnap, setBootstrapSnap] = useState<ClusterBootstrapSnapshot | null>(null);

  const loadSettings = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      setSettings(
        r.e2bPlatform ?? {
          readOnly: true,
          e2bApiUrl: "",
          e2bDomain: "",
          apiKeySet: false,
          workerStrictTemplate: "",
          workerRelaxedTemplate: "",
          sandboxTimeoutSecs: 3600,
          configured: false,
        }
      );
      setE2bWorker(r.e2bWorker ?? null);
    } finally {
      setLoading(false);
    }
  }, [gatewayBase]);

  const refreshBootstrap = useCallback(async (): Promise<ClusterBootstrapSnapshot | null> => {
    if (!gatewayBase) return null;
    try {
      const data = await proxyHttp<ClusterBootstrapSnapshot>(
        gatewayBase,
        "GET",
        "/v1/gateway/bootstrap/status"
      );
      setBootstrapSnap(data);
      return data;
    } catch {
      return null;
    }
  }, [gatewayBase]);

  useEffect(() => {
    void loadSettings();
    void refreshBootstrap();
  }, [loadSettings, refreshBootstrap]);

  const reloadAll = async () => {
    await Promise.all([loadSettings(), refreshBootstrap()]);
  };

  return (
    <Space direction="vertical" size="large" style={{ width: "100%" }}>
      <Space style={{ width: "100%", justifyContent: "space-between" }}>
        <Typography.Title level={4} style={{ margin: 0 }}>
          <ApiOutlined /> e2b 平台
        </Typography.Title>
        <Button icon={<ReloadOutlined />} loading={loading} onClick={() => void reloadAll()}>
          刷新
        </Button>
      </Space>

      <Alert
        type="info"
        showIcon
        message="连接只读；模板可随时重打"
        description={
          <Typography.Paragraph style={{ marginBottom: 0 }}>
            e2b 平台地址与密钥来自仓库 <Typography.Text code>.env</Typography.Text>
            ，改完需重启 Gateway。下方「制作 / 升级模板」与集群 Init 同一路径：选 ACR/CI tag →
            异步发布 worker / relaxed（含 OVS）/ observe / nas-api，无需重建 gateway 镜像。
          </Typography.Paragraph>
        }
      />

      <Card title="当前 e2b 连接（只读）" loading={loading}>
        {settings ? (
          <Descriptions column={1} bordered size="small">
            <Descriptions.Item label="CLAW_E2B_API_URL">
              <Typography.Text
                code
                copyable={settings.e2bApiUrl ? { text: settings.e2bApiUrl } : undefined}
              >
                {settings.e2bApiUrl || "（未设置）"}
              </Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="CLAW_E2B_SANDBOX_URL">
              <Typography.Text
                code
                copyable={
                  settings.e2bSandboxUrl ? { text: settings.e2bSandboxUrl } : undefined
                }
              >
                {settings.e2bSandboxUrl || "（未设置，使用 API 默认）"}
              </Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="CLAW_E2B_DOMAIN">
              <Typography.Text code>{settings.e2bDomain || "（未设置）"}</Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="CLAW_E2B_API_KEY">
              <Tag color={settings.apiKeySet ? "green" : "red"}>
                {settings.apiKeySet ? "已配置" : "未配置"}
              </Tag>
            </Descriptions.Item>
            <Descriptions.Item label="worker strict 模板">
              <Typography.Text code>{settings.workerStrictTemplate}</Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="Strict poolSize（PG）">
              <Typography.Text code>{e2bWorker?.poolSize ?? 1}</Typography.Text>
              {e2bWorker?.poolSizeCap != null ? (
                <Typography.Text type="secondary" style={{ marginLeft: 8 }}>
                  cap={e2bWorker.poolSizeCap}
                </Typography.Text>
              ) : null}
              <Typography.Text type="secondary" style={{ marginLeft: 8 }}>
                在「核心组件」页修改
              </Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="relaxedWorkerAllowed">
              <Tag color={settings.relaxedWorkerAllowed === false ? "red" : "green"}>
                {settings.relaxedWorkerAllowed === false ? "false（严格模式）" : "true"}
              </Tag>
            </Descriptions.Item>
            <Descriptions.Item label="worker relaxed 模板">
              <Typography.Text code>{settings.workerRelaxedTemplate}</Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="sandbox TTL（秒）">
              <Typography.Text code>{settings.sandboxTimeoutSecs}</Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="configured">
              <Tag color={settings.configured ? "green" : "default"}>
                {settings.configured ? "API URL + Key 就绪" : "未就绪"}
              </Tag>
            </Descriptions.Item>
          </Descriptions>
        ) : null}
      </Card>

      <Card title="制作 / 升级 e2b 模板">
        {bootstrapSnap ? (
          <TemplateBuildStep
            mode="ops"
            snap={bootstrapSnap}
            onRefresh={refreshBootstrap}
          />
        ) : (
          <Typography.Text type="secondary">加载 bootstrap 状态中…</Typography.Text>
        )}
      </Card>
    </Space>
  );
}
