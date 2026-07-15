import { ApiOutlined, ReloadOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Descriptions, Space, Tag, Typography } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type { E2bPlatformSettings, E2bWorkerSettings, GlobalSettingsResponse } from "../../types/globalSettings";

/** Admin read-only e2b platform view (source: repo `.env`, restart gateway to apply). Author: kejiqing */
export default function E2bPlatformPage() {
  const { gatewayBase } = useApp();
  const [loading, setLoading] = useState(false);
  const [settings, setSettings] = useState<E2bPlatformSettings | null>(null);
  const [e2bWorker, setE2bWorker] = useState<E2bWorkerSettings | null>(null);

  const load = useCallback(async () => {
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

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <Space direction="vertical" size="large" style={{ width: "100%" }}>
      <Space style={{ width: "100%", justifyContent: "space-between" }}>
        <Typography.Title level={4} style={{ margin: 0 }}>
          <ApiOutlined /> e2b 平台
        </Typography.Title>
        <Button icon={<ReloadOutlined />} loading={loading} onClick={() => void load()}>
          刷新
        </Button>
      </Space>

      <Alert
        type="warning"
        showIcon
        message="只读展示"
        description={
          <Typography.Paragraph style={{ marginBottom: 0 }}>
            e2b 平台地址与密钥通过仓库根目录 <Typography.Text code>.env</Typography.Text>{" "}
            配置，修改后需重启 Gateway 生效。Admin 不提供运行时切换 e2b 的能力。
            <br />
            关键变量：<Typography.Text code>CLAW_E2B_API_URL</Typography.Text>、
            <Typography.Text code>CLAW_E2B_SANDBOX_URL</Typography.Text>、
            <Typography.Text code>CLAW_E2B_API_KEY</Typography.Text>
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
    </Space>
  );
}
