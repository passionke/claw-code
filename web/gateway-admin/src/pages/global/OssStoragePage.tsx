import { CloudUploadOutlined, ReloadOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Descriptions, Space, Tag, Typography } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type { GlobalSettingsResponse, OssStorageSettings } from "../../types/globalSettings";

/** Admin read-only OSS attachment store (repo `.env` CLAW_OSS_*). Author: kejiqing */
export default function OssStoragePage() {
  const { gatewayBase } = useApp();
  const [loading, setLoading] = useState(false);
  const [settings, setSettings] = useState<OssStorageSettings | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      setSettings(
        r.oss ?? {
          enabled: false,
          endpoint: "",
          region: "",
          bucket: "",
          keyPrefix: "sessions",
          accessKeyIdSet: false,
          objectTtlDays: 730,
          signedUrlTtlSecs: 3600,
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
          <CloudUploadOutlined /> OSS 附件存储
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
            会话附件双写配置通过仓库根目录 <Typography.Text code>.env</Typography.Text> 的{" "}
            <Typography.Text code>CLAW_OSS_*</Typography.Text> 维护，修改后需重启 Gateway。Admin
            不提供保存入口；SK 从不回传。
            <br />
            对象过期由桶 lifecycle（前缀{" "}
            <Typography.Text code>sessions/</Typography.Text>）执行，天数与{" "}
            <Typography.Text code>CLAW_OSS_OBJECT_TTL_DAYS</Typography.Text> 对齐。
          </Typography.Paragraph>
        }
      />

      <Card loading={loading}>
        {settings ? (
          <Descriptions column={1} bordered size="small">
            <Descriptions.Item label="enabled">
              {settings.enabled ? <Tag color="success">已启用</Tag> : <Tag>未启用</Tag>}
            </Descriptions.Item>
            <Descriptions.Item label="endpoint">{settings.endpoint || "—"}</Descriptions.Item>
            <Descriptions.Item label="region">{settings.region || "—"}</Descriptions.Item>
            <Descriptions.Item label="bucket">{settings.bucket || "—"}</Descriptions.Item>
            <Descriptions.Item label="keyPrefix">{settings.keyPrefix || "—"}</Descriptions.Item>
            <Descriptions.Item label="accessKeyIdSet">
              {settings.accessKeyIdSet ? <Tag color="blue">已配置</Tag> : <Tag>未配置</Tag>}
            </Descriptions.Item>
            <Descriptions.Item label="objectTtlDays">{settings.objectTtlDays}</Descriptions.Item>
            <Descriptions.Item label="signedUrlTtlSecs">
              {settings.signedUrlTtlSecs}
            </Descriptions.Item>
          </Descriptions>
        ) : null}
      </Card>
    </Space>
  );
}
