import { CloudServerOutlined, ReloadOutlined, SaveOutlined, SyncOutlined } from "@ant-design/icons";
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
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type {
  E2bSingletonActionResponse,
  E2bSingletonsStatusResponse,
  GlobalSettingsResponse,
  PutE2bSingletonTemplatesResponse,
} from "../../types/globalSettings";

function formatMs(ms?: number): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

function onlineTag(online: boolean, label: string) {
  return <Tag color={online ? "success" : "warning"}>{online ? `${label} 在线` : `${label} 未就绪`}</Tag>;
}

/** Gateway 核心 e2b 单例：nas-api / observe；OVS 内置在 relaxed worker。Author: kejiqing */
export default function E2bCoreComponentsPage() {
  const { gatewayBase } = useApp();
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [resetting, setResetting] = useState<string | null>(null);
  const [status, setStatus] = useState<E2bSingletonsStatusResponse | null>(null);
  const [clusterId, setClusterId] = useState("");
  const [form] = Form.useForm<{
    nasApiTemplateId: string;
    observeTemplateId: string;
  }>();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [gs, singletons] = await Promise.all([
        proxyHttp<GlobalSettingsResponse>(gatewayBase, "GET", "/v1/gateway/global-settings"),
        proxyHttp<E2bSingletonsStatusResponse>(
          gatewayBase,
          "GET",
          "/v1/gateway/global-settings/e2b-singletons"
        ),
      ]);
      setClusterId(gs.clusterId ?? "");
      setStatus(singletons);
      form.setFieldsValue({
        nasApiTemplateId: singletons.nasApi.templateId ?? singletons.nasApi.effectiveTemplateId,
        observeTemplateId:
          singletons.observe.templateId ?? singletons.observe.effectiveTemplateId,
      });
    } catch (e) {
      message.error(`加载核心组件失败：${String(e)}`);
    } finally {
      setLoading(false);
    }
  }, [form, gatewayBase]);

  useEffect(() => {
    void load();
  }, [load]);

  const saveTemplates = async () => {
    const values = await form.validateFields();
    setSaving(true);
    try {
      const r = await proxyHttp<PutE2bSingletonTemplatesResponse>(
        gatewayBase,
        "PUT",
        "/v1/gateway/global-settings/e2b-singleton-templates",
        {
          nasApiTemplateId: values.nasApiTemplateId.trim(),
          observeTemplateId: values.observeTemplateId.trim(),
        }
      );
      setStatus({
        nasApi: r.nasApi,
        ovs: r.ovs,
        observe: r.observe,
      });
      message.success("模版 ID 已写入 PG");
    } catch (e) {
      message.error(`保存模版 ID 失败：${String(e)}`);
    } finally {
      setSaving(false);
    }
  };

  const resetComponent = async (component: "nas-api" | "observe") => {
    setResetting(component);
    try {
      const r = await proxyHttp<E2bSingletonActionResponse>(
        gatewayBase,
        "POST",
        `/v1/gateway/global-settings/e2b-singletons/${component}/reset`
      );
      if (r.trafficReachable) {
        message.success(`${component} 已重置（${r.sandboxId ?? "—"}）`);
      } else {
        message.warning(r.message ?? `${component} 已重建，但 traffic 探测未通过`);
      }
      await load();
    } catch (e) {
      message.error(`重置 ${component} 失败：${String(e)}`);
    } finally {
      setResetting(null);
    }
  };

  return (
    <Space direction="vertical" size="large" style={{ width: "100%", maxWidth: 960 }}>
      <Space style={{ width: "100%", justifyContent: "space-between" }}>
        <Typography.Title level={4} style={{ margin: 0 }}>
          <CloudServerOutlined /> Gateway 核心组件
        </Typography.Title>
        <Button icon={<ReloadOutlined />} loading={loading} onClick={() => void load()}>
          刷新
        </Button>
      </Space>

      <Alert
        type="info"
        showIcon
        message="nas-api / observe 单例由 Gateway 统一掌控"
        description={
          <Typography.Paragraph style={{ marginBottom: 0 }}>
            nas-api、observe 在 gateway 启动时自动 ensure，健康检查失败时自动重建。
            OVS 已内置在 relaxed project worker（<Typography.Text code>claw-worker-relaxed</Typography.Text>
            ），不再维护集群级 OVS singleton。
            模版 ID 注册在 PG（<Typography.Text code>settings_json</Typography.Text>
            ），新模版构建后在此更新并点「重置」即可滚动升级。
            <br />
            集群 ID：<Typography.Text code>{clusterId || "—"}</Typography.Text>
          </Typography.Paragraph>
        }
      />

      <Card title="模版 ID（PG）" loading={loading}>
        <Form form={form} layout="vertical">
          <Form.Item
            label="nas-api templateId"
            name="nasApiTemplateId"
            extra={
              status ? (
                <Typography.Text type="secondary">
                  当前生效：{status.nasApi.effectiveTemplateId}
                </Typography.Text>
              ) : null
            }
          >
            <Input placeholder="claw-nas-api" />
          </Form.Item>
          <Form.Item
            label="observe templateId"
            name="observeTemplateId"
            extra={
              status ? (
                <Typography.Text type="secondary">
                  当前生效：{status.observe.effectiveTemplateId}
                </Typography.Text>
              ) : null
            }
          >
            <Input placeholder="claw-observe" />
          </Form.Item>
          <Button
            type="primary"
            icon={<SaveOutlined />}
            loading={saving}
            onClick={() => void saveTemplates()}
          >
            保存模版 ID
          </Button>
        </Form>
      </Card>

      <Card title="nas-api" loading={loading}>
        {status ? (
          <>
            <Descriptions column={1} bordered size="small">
              <Descriptions.Item label="状态">
                {onlineTag(status.nasApi.online, "nas-api")}
              </Descriptions.Item>
              <Descriptions.Item label="sandboxId">
                {status.nasApi.sandboxId ? (
                  <Typography.Text code copyable>
                    {status.nasApi.sandboxId}
                  </Typography.Text>
                ) : (
                  "—"
                )}
              </Descriptions.Item>
              <Descriptions.Item label="baseUrl">
                {status.nasApi.baseUrl ? (
                  <Typography.Text code copyable>
                    {status.nasApi.baseUrl}
                  </Typography.Text>
                ) : (
                  "—"
                )}
              </Descriptions.Item>
              <Descriptions.Item label="更新时间">
                {formatMs(status.nasApi.updatedAtMs)}
              </Descriptions.Item>
            </Descriptions>
            <Popconfirm
              title="重置 nas-api 单例？"
              description="将删除当前 sandbox 并按 PG 模版 ID 重建。"
              onConfirm={() => void resetComponent("nas-api")}
              okText="重置"
              cancelText="取消"
            >
              <Button
                style={{ marginTop: 16 }}
                icon={<SyncOutlined />}
                loading={resetting === "nas-api"}
              >
                重置 nas-api
              </Button>
            </Popconfirm>
          </>
        ) : null}
      </Card>

      <Card title="observe（Tap 代理 + Live）" loading={loading}>
        {status ? (
          <>
            <Descriptions column={1} bordered size="small">
              <Descriptions.Item label="模版">
                <Typography.Text code>{status.observe.effectiveTemplateId}</Typography.Text>
              </Descriptions.Item>
              <Descriptions.Item label="模版更新时间">
                {formatMs(status.observe.updatedAtMs)}
              </Descriptions.Item>
            </Descriptions>
            <Popconfirm
              title="重置 observe 单例？"
              description="将删除当前 observe sandbox 并重建 claude-tap（约 1–2 分钟）。"
              onConfirm={() => void resetComponent("observe")}
              okText="重置"
              cancelText="取消"
            >
              <Button
                style={{ marginTop: 16 }}
                type="primary"
                icon={<SyncOutlined />}
                loading={resetting === "observe"}
              >
                重置 observe
              </Button>
            </Popconfirm>
            <Typography.Paragraph type="secondary" style={{ marginTop: 8, marginBottom: 0 }}>
              代理/Live 详情见「全局推理」页。
            </Typography.Paragraph>
          </>
        ) : null}
      </Card>

      <Card title="ovs（已废弃集群单例）" loading={loading}>
        {status ? (
          <>
            <Alert
              type="warning"
              showIcon
              message="OVS 运行在 relaxed project worker 内"
              description={
                status.ovs.migrationNote ??
                "请使用各项目 OVS workspace API，不再创建或重置集群级 OVS singleton。"
              }
              style={{ marginBottom: 16 }}
            />
            <Descriptions column={1} bordered size="small">
              <Descriptions.Item label="运行方式">
                <Typography.Text code>{status.ovs.effectiveTemplateId}</Typography.Text>
              </Descriptions.Item>
              <Descriptions.Item label="legacy sandboxId">
                {status.ovs.sandboxId ? (
                  <Typography.Text code copyable>
                    {status.ovs.sandboxId}
                  </Typography.Text>
                ) : (
                  "—"
                )}
              </Descriptions.Item>
              <Descriptions.Item label="legacy baseUrl">
                {status.ovs.baseUrl ? (
                  <Typography.Text code copyable>
                    {status.ovs.baseUrl}
                  </Typography.Text>
                ) : (
                  "—"
                )}
              </Descriptions.Item>
              <Descriptions.Item label="PG 更新时间">
                {formatMs(status.ovs.updatedAtMs)}
              </Descriptions.Item>
            </Descriptions>
          </>
        ) : null}
      </Card>
    </Space>
  );
}
