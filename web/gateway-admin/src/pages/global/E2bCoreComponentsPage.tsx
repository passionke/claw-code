import { CloudServerOutlined, ReloadOutlined, SaveOutlined, SyncOutlined } from "@ant-design/icons";
import {
  Alert,
  Button,
  Card,
  Descriptions,
  Form,
  InputNumber,
  Popconfirm,
  Select,
  Space,
  Tag,
  Typography,
  message,
} from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type {
  E2bSingletonActionResponse,
  E2bSingletonsStatusResponse,
  E2bTemplateEntry,
  E2bTemplatesListResponse,
  E2bWorkerSettings,
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

function boolTag(ok?: boolean, yes = "是", no = "否") {
  if (ok === undefined) return "—";
  return <Tag color={ok ? "success" : "error"}>{ok ? yes : no}</Tag>;
}

function templateSelectLabel(t: E2bTemplateEntry): string {
  const alias = t.aliases[0] ?? t.templateId;
  const ready = t.imagePresent ? "" : "（镜像未就绪）";
  return `${alias} · ${t.templateId}${ready}`;
}

function templatesForAlias(templates: E2bTemplateEntry[], alias: string): E2bTemplateEntry[] {
  return templates.filter((t) => t.aliases.includes(alias));
}

function preferredTemplateId(
  pgTemplateId: string | undefined,
  effectiveId: string,
  templates: E2bTemplateEntry[],
  alias: string
): string {
  const fromPg = pgTemplateId?.trim();
  if (fromPg?.startsWith("tpl_")) return fromPg;
  const fromEffective = effectiveId.trim();
  if (fromEffective.startsWith("tpl_")) return fromEffective;
  const ready = templatesForAlias(templates, alias).filter((t) => t.imagePresent);
  if (ready.length > 0) return ready[0].templateId;
  return fromPg || fromEffective;
}

function templateSelectOptions(
  templates: E2bTemplateEntry[],
  alias: string,
  currentValue?: string
): { value: string; label: string; disabled?: boolean }[] {
  const filtered = templatesForAlias(templates, alias);
  const seen = new Set<string>();
  const options = filtered.map((t) => {
    seen.add(t.templateId);
    return {
      value: t.templateId,
      label: templateSelectLabel(t),
      disabled: !t.imagePresent,
    };
  });
  if (currentValue && !seen.has(currentValue)) {
    options.unshift({
      value: currentValue,
      label: `${currentValue}（PG 当前，e2bserver 未列出）`,
      disabled: false,
    });
  }
  return options;
}

/** Gateway 核心 e2b 组件：nas-api / observe（OVS 已迁至 relaxed worker）。Author: kejiqing */
export default function E2bCoreComponentsPage() {
  const { gatewayBase } = useApp();
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [savingWorker, setSavingWorker] = useState(false);
  const [resetting, setResetting] = useState<string | null>(null);
  const [status, setStatus] = useState<E2bSingletonsStatusResponse | null>(null);
  const [e2bWorker, setE2bWorker] = useState<E2bWorkerSettings | null>(null);
  const [e2bTemplates, setE2bTemplates] = useState<E2bTemplateEntry[]>([]);
  const [e2bApiUrl, setE2bApiUrl] = useState("");
  const [clusterId, setClusterId] = useState("");
  const [poolSizeCap, setPoolSizeCap] = useState(16);
  const [form] = Form.useForm<{
    nasApiTemplateId: string;
    observeTemplateId: string;
  }>();
  const [workerForm] = Form.useForm<{
    workerTemplateId: string;
    poolSize: number;
  }>();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [gs, singletons, templatesResp] = await Promise.all([
        proxyHttp<GlobalSettingsResponse>(gatewayBase, "GET", "/v1/gateway/global-settings"),
        proxyHttp<E2bSingletonsStatusResponse>(
          gatewayBase,
          "GET",
          "/v1/gateway/global-settings/e2b-singletons"
        ),
        proxyHttp<E2bTemplatesListResponse>(
          gatewayBase,
          "GET",
          "/v1/gateway/global-settings/e2b-templates"
        ).catch(() => null),
      ]);
      setClusterId(gs.clusterId ?? "");
      setE2bWorker(gs.e2bWorker ?? null);
      setPoolSizeCap(gs.e2bWorker?.poolSizeCap ?? 16);
      setStatus(singletons);
      const templates = templatesResp?.templates ?? [];
      if (templatesResp) {
        setE2bTemplates(templates);
        setE2bApiUrl(templatesResp.apiUrl);
      }
      form.setFieldsValue({
        nasApiTemplateId: preferredTemplateId(
          singletons.nasApi.templateId,
          singletons.nasApi.effectiveTemplateId,
          templates,
          "claw-nas-api"
        ),
        observeTemplateId: preferredTemplateId(
          singletons.observe.templateId,
          singletons.observe.effectiveTemplateId,
          templates,
          "claw-observe"
        ),
      });
      workerForm.setFieldsValue({
        workerTemplateId: preferredTemplateId(
          gs.e2bWorker?.templateId,
          gs.e2bPlatform?.workerStrictTemplate ?? "claw-worker",
          templates,
          "claw-worker"
        ),
        poolSize: gs.e2bWorker?.poolSize ?? 1,
      });
    } catch (e) {
      message.error(`加载核心组件失败：${String(e)}`);
    } finally {
      setLoading(false);
    }
  }, [form, workerForm, gatewayBase]);

  useEffect(() => {
    void load();
  }, [load]);

  const nasApiTemplateOptions = useMemo(
    () =>
      templateSelectOptions(
        e2bTemplates,
        "claw-nas-api",
        status?.nasApi.templateId ?? status?.nasApi.effectiveTemplateId
      ),
    [e2bTemplates, status]
  );

  const observeTemplateOptions = useMemo(
    () =>
      templateSelectOptions(
        e2bTemplates,
        "claw-observe",
        status?.observe.templateId ?? status?.observe.effectiveTemplateId
      ),
    [e2bTemplates, status]
  );

  const workerTemplateOptions = useMemo(
    () =>
      templateSelectOptions(
        e2bTemplates,
        "claw-worker",
        e2bWorker?.templateId
      ),
    [e2bTemplates, e2bWorker]
  );

  const saveWorkerSettings = async () => {
    const values = await workerForm.validateFields();
    setSavingWorker(true);
    try {
      const r = await proxyHttp<E2bWorkerSettings>(
        gatewayBase,
        "PUT",
        "/v1/gateway/global-settings/e2b-worker",
        {
          templateId: values.workerTemplateId.trim(),
          poolSize: values.poolSize,
        }
      );
      setE2bWorker(r);
      message.success("Strict Worker 池配置已保存；Gateway 将后台 reconcile");
    } catch (e) {
      message.error(`保存 Strict Worker 配置失败：${String(e)}`);
    } finally {
      setSavingWorker(false);
    }
  };

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
        message="生命周期由 Gateway 统一掌控"
        description={
          <Typography.Paragraph style={{ marginBottom: 0 }}>
            nas-api、observe 两个核心组件由 gateway 启动时自动 ensure，健康检查失败时自动重建。
            模版 ID 注册在 PG（<Typography.Text code>settings_json</Typography.Text>
            ），新模版构建后在此更新并点「重置」即可滚动升级。
            <br />
            集群 ID：<Typography.Text code>{clusterId || "—"}</Typography.Text>
          </Typography.Paragraph>
        }
      />

      <Card
        title="模版 ID（PG）"
        loading={loading}
        extra={
          e2bApiUrl ? (
            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
              e2bserver: <Typography.Text code>{e2bApiUrl}</Typography.Text>
            </Typography.Text>
          ) : null
        }
      >
        <Form form={form} layout="vertical">
          <Form.Item
            label="nas-api templateId"
            name="nasApiTemplateId"
            rules={[{ required: true, message: "请选择 nas-api 模版" }]}
            extra={
              status ? (
                <Typography.Text type="secondary">
                  当前生效：{status.nasApi.effectiveTemplateId}
                  {templatesForAlias(e2bTemplates, "claw-nas-api").length === 0
                    ? "（e2bserver 未返回 claw-nas-api 模版）"
                    : null}
                </Typography.Text>
              ) : null
            }
          >
            <Select
              showSearch
              placeholder="从 e2bserver 选择 claw-nas-api 模版"
              optionFilterProp="label"
              options={nasApiTemplateOptions}
              notFoundContent="e2bserver 无 claw-nas-api 模版，请先 build"
            />
          </Form.Item>
          <Form.Item
            label="observe templateId"
            name="observeTemplateId"
            rules={[{ required: true, message: "请选择 observe 模版" }]}
            extra={
              status ? (
                <Typography.Text type="secondary">
                  当前生效：{status.observe.effectiveTemplateId}
                  {templatesForAlias(e2bTemplates, "claw-observe").length === 0
                    ? "（e2bserver 未返回 claw-observe 模版）"
                    : null}
                </Typography.Text>
              ) : null
            }
          >
            <Select
              showSearch
              placeholder="从 e2bserver 选择 claw-observe 模版"
              optionFilterProp="label"
              options={observeTemplateOptions}
              notFoundContent="e2bserver 无 claw-observe 模版，请先 build"
            />
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

      <Card
        title="Strict Worker（PG）"
        loading={loading}
        extra={
          e2bWorker?.updatedAtMs ? (
            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
              更新于 {formatMs(e2bWorker.updatedAtMs)}
            </Typography.Text>
          ) : null
        }
      >
        <Alert
          type="info"
          showIcon
          style={{ marginBottom: 16 }}
          message="每 strict 项目的 solve worker 池（全局默认）"
          description="poolSize 为全局默认；单项目可在「Worker 执行环境」用 workerProfileJson.poolSize 覆盖。变更后 Gateway 后台 reconcile；缩容需等待各 slot lease 清空。上限来自 .env 的 CLAW_E2B_POOL_SIZE_CAP（超过会报错）。"
        />
        <Form form={workerForm} layout="vertical">
          <Form.Item
            label="claw-worker templateId"
            name="workerTemplateId"
            rules={[{ required: true, message: "请选择 strict worker 模版" }]}
          >
            <Select
              showSearch
              placeholder="从 e2bserver 选择 claw-worker 模版"
              optionFilterProp="label"
              options={workerTemplateOptions}
              notFoundContent="e2bserver 无 claw-worker 模版，请先 build"
            />
          </Form.Item>
          <Form.Item
            label="poolSize（全局默认 · 每 strict 项目）"
            name="poolSize"
            rules={[{ required: true, type: "number", min: 1, max: poolSizeCap }]}
            extra={`默认 1，范围 1–${poolSizeCap}（CLAW_E2B_POOL_SIZE_CAP）。relaxed 项目固定 1 worker。`}
          >
            <InputNumber min={1} max={poolSizeCap} style={{ width: 120 }} />
          </Form.Item>
          <Button
            type="primary"
            icon={<SaveOutlined />}
            loading={savingWorker}
            onClick={() => void saveWorkerSettings()}
          >
            保存 Strict Worker 配置
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
              <Descriptions.Item label="状态">
                {onlineTag(Boolean(status.observe.healthy), "observe")}
              </Descriptions.Item>
              <Descriptions.Item label="模版">
                <Typography.Text code>{status.observe.effectiveTemplateId}</Typography.Text>
              </Descriptions.Item>
              <Descriptions.Item label="sandboxId">
                {status.observe.sandboxId ? (
                  <Typography.Text code copyable>
                    {status.observe.sandboxId}
                  </Typography.Text>
                ) : (
                  "—"
                )}
              </Descriptions.Item>
              <Descriptions.Item label="Live URL">
                {status.observe.baseUrl ? (
                  <Typography.Text code copyable>
                    {status.observe.baseUrl}
                  </Typography.Text>
                ) : (
                  "—"
                )}
              </Descriptions.Item>
              <Descriptions.Item label="运行中">{boolTag(status.observe.running)}</Descriptions.Item>
              <Descriptions.Item label="可达">{boolTag(status.observe.reachable)}</Descriptions.Item>
              <Descriptions.Item label="模版更新时间">
                {formatMs(status.observe.updatedAtMs)}
              </Descriptions.Item>
              <Descriptions.Item label="最近检查">
                {formatMs(status.observe.lastCheckedAtMs)}
              </Descriptions.Item>
              <Descriptions.Item label="最近错误">
                {status.observe.lastError || "—"}
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
    </Space>
  );
}
