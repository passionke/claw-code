import { CloudServerOutlined, ReloadOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Descriptions, Space, Tag, Typography } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type { E2bNasSettings, GlobalSettingsResponse } from "../../types/globalSettings";

/** Admin read-only e2b NAS view (source: repo `.env`, restart gateway to apply). Author: kejiqing */
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
          nasHostMount: "",
          e2bNasServer: "",
          e2bNasExport: "",
          configured: false,
          gatewayWorkRoot: "/var/lib/claw/workspace",
          nasRootResolved: "",
          layoutActive: false,
          pathExists: false,
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
        type="warning"
        showIcon
        message="只读展示"
        description={
          <Typography.Paragraph style={{ marginBottom: 0 }}>
            NAS 配置通过仓库根目录 <Typography.Text code>.env</Typography.Text> 维护，修改后需重启
            Gateway 生效。Admin 不提供保存入口。
            <br />
            关键变量：<Typography.Text code>CLAW_NAS_HOST_MOUNT</Typography.Text>、
            <Typography.Text code>CLAW_E2B_NAS_SERVER</Typography.Text>、
            <Typography.Text code>CLAW_E2B_NAS_EXPORT</Typography.Text>、
            <Typography.Text code>CLAW_WORK_ROOT</Typography.Text>
          </Typography.Paragraph>
        }
      />

      <Alert
        type="info"
        showIcon
        message="三层 NAS 映射"
        description={
          <Typography.Paragraph style={{ marginBottom: 0 }}>
            ① 宿主机 NFS → <Typography.Text code>/mnt/nas0</Typography.Text>（ECS）或{" "}
            <Typography.Text code>/Volumes/claw-nas</Typography.Text>（Mac）
            <br />
            ② Gateway 容器：<Typography.Text code>CLAW_NAS_HOST_MOUNT</Typography.Text> bind →{" "}
            <Typography.Text code>{settings?.gatewayWorkRoot ?? "CLAW_WORK_ROOT"}</Typography.Text>
            <br />
            ③ e2b sandbox：创建时静态 bind <Typography.Text code>proj_N/workers/…</Typography.Text> →{" "}
            <Typography.Text code>/claw_host_root</Typography.Text>
          </Typography.Paragraph>
        }
      />

      <Card title="当前环境变量（只读）" loading={loading}>
        {settings ? (
          <Descriptions column={1} bordered size="small">
            <Descriptions.Item label="CLAW_NAS_HOST_MOUNT">
              <Typography.Text code copyable={settings.nasHostMount ? { text: settings.nasHostMount } : undefined}>
                {settings.nasHostMount || "（未设置）"}
              </Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="CLAW_E2B_NAS_SERVER">
              <Typography.Text code>{settings.e2bNasServer || "（未设置）"}</Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="CLAW_E2B_NAS_EXPORT">
              <Typography.Text code>{settings.e2bNasExport || "（未设置）"}</Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="CLAW_WORK_ROOT">
              <Typography.Text code>{settings.gatewayWorkRoot}</Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="nasRootResolved">
              <Typography.Text code>{settings.nasRootResolved || "—"}</Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="Gateway 可见路径">
              <Tag color={settings.pathExists ? "green" : "red"}>
                {settings.pathExists ? "存在" : "不可见"}
              </Tag>
            </Descriptions.Item>
            {settings.hasProjTree != null ? (
              <Descriptions.Item label="proj 目录树">
                <Tag color={settings.hasProjTree ? "green" : "default"}>
                  {settings.hasProjTree ? "已有" : "无"}
                </Tag>
              </Descriptions.Item>
            ) : null}
            <Descriptions.Item label="layoutActive">
              <Tag color={settings.layoutActive ? "green" : "red"}>
                {settings.layoutActive ? "active" : "inactive"}
              </Tag>
            </Descriptions.Item>
            <Descriptions.Item label="configured">
              <Tag color={settings.configured ? "green" : "default"}>
                {settings.configured ? "CLAW_NAS_HOST_MOUNT 已设" : "未配置"}
              </Tag>
            </Descriptions.Item>
          </Descriptions>
        ) : null}
      </Card>
    </Space>
  );
}
