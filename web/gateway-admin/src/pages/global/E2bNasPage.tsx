import { CloudServerOutlined, ReloadOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Descriptions, Space, Tag, Typography } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type { E2bNasSettings, GlobalSettingsResponse } from "../../types/globalSettings";

/** Admin read-only e2b NAS view (e2b GET /health + nas-api). Author: kejiqing */
export default function E2bNasPage() {
  const { gatewayBase } = useApp();
  const [loading, setLoading] = useState(false);
  const [settings, setSettings] = useState<E2bNasSettings | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      setSettings(
        r.e2bNas ?? {
          readOnly: true,
          e2bHostMountRoot: "",
          e2bNasReady: false,
          configured: false,
          nasApiEnabled: true,
          layoutActive: false,
        }
      );
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
          <CloudServerOutlined /> e2b NAS 存储
        </Typography.Title>
        <Button icon={<ReloadOutlined />} loading={loading} onClick={() => void load()}>
          刷新
        </Button>
      </Space>

      <Alert
        type="info"
        showIcon
        message="Gateway 不配置 e2b 宿主机 bind 根"
        description={
          <Typography.Paragraph style={{ marginBottom: 0 }}>
            workspace 逻辑路径由 Gateway 经 claw-nas-api 读写；e2b 宿主机 NAS bind / traffic 在
            e2bserver <Typography.Text code>config/deploy.toml</Typography.Text> 维护。下方数据来自 e2b{" "}
            <Typography.Text code>GET /health</Typography.Text>。
          </Typography.Paragraph>
        }
      />

      <Card title="e2b 平台 NAS（只读）" loading={loading}>
        {settings ? (
          <Descriptions column={1} bordered size="small">
            <Descriptions.Item label="e2b hostMountRoot">
              <Typography.Text
                code
                copyable={
                  settings.e2bHostMountRoot ? { text: settings.e2bHostMountRoot } : undefined
                }
              >
                {settings.e2bHostMountRoot || "（e2b /health 未回报）"}
              </Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="sandboxInject">
              <Typography.Text code>{settings.sandboxInject || "—"}</Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="mountSource">
              <Typography.Text code>{settings.mountSource || "—"}</Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="e2b NAS ready">
              <Tag color={settings.e2bNasReady ? "green" : "red"}>
                {settings.e2bNasReady ? "ready" : "not ready"}
              </Tag>
            </Descriptions.Item>
            <Descriptions.Item label="nas-api">
              <Tag color={settings.nasApiEnabled ? "green" : "default"}>
                {settings.nasApiEnabled ? "enabled" : "disabled"}
              </Tag>
            </Descriptions.Item>
            <Descriptions.Item label="layoutActive">
              <Tag color={settings.layoutActive ? "green" : "red"}>
                {settings.layoutActive ? "active" : "inactive"}
              </Tag>
            </Descriptions.Item>
            <Descriptions.Item label="configured">
              <Tag color={settings.configured ? "green" : "default"}>
                {settings.configured ? "就绪" : "未就绪"}
              </Tag>
            </Descriptions.Item>
          </Descriptions>
        ) : null}
      </Card>
    </Space>
  );
}
