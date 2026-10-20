import {
  Alert,
  Button,
  Card,
  Descriptions,
  Form,
  Input,
  InputNumber,
  Popconfirm,
  Radio,
  Select,
  Space,
  Spin,
  Switch,
  Table,
  Tag,
  Typography,
  message,
} from "antd";
import { MinusCircleOutlined, PlusOutlined, ReloadOutlined } from "@ant-design/icons";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import { putProjectConfigDraft } from "../utils/projectConfig";
import type { GlobalSettingsResponse } from "../types/globalSettings";
import type {
  LandlockDsl,
  WorkerProfileJson,
  WorkerVolumeMountJson,
} from "../types/landlock";
import {
  normalizeWorkerResourcesClient,
  validateLandlockDslClient,
  workerResourcesFingerprint,
  WORKER_CPU_MAX,
  WORKER_MEMORY_MAX_MB,
  WORKER_MEMORY_QUANTUM_MB,
} from "../types/landlock";
import type {
  ProjectE2bWorkerInfo,
  ProjectE2bWorkerResetResponse,
  ProjectE2bWorkerStatusResponse,
} from "../types/projectE2bWorker";

type Mode = WorkerProfileJson["mode"];

type E2bVolumeCatalogEntry = {
  volumeID: string;
  name: string;
};

type FormValues = {
  mode: Mode;
  poolSize?: number | null;
  askUserQuestionInAgent?: boolean;
  resourcesCustom: boolean;
  cpuCount?: number | null;
  memoryMb?: number | null;
  landlockInherit: boolean;
  landlockEnabled: boolean;
  landlockRw: string[];
  landlockRo: string[];
};

export default function WorkerProfilePage() {
  const { gatewayBase, projId, projectConfig, refreshProjectConfig } = useApp();
  const [form] = Form.useForm<FormValues>();
  const [systemDefault, setSystemDefault] = useState<LandlockDsl | null>(null);
  const [relaxedAllowed, setRelaxedAllowed] = useState(true);
  const [poolSizeCap, setPoolSizeCap] = useState(16);
  const [globalPoolSize, setGlobalPoolSize] = useState(1);
  const [workerStatus, setWorkerStatus] = useState<ProjectE2bWorkerStatusResponse | null>(null);
  const [workerLoading, setWorkerLoading] = useState(false);
  const [workerResetting, setWorkerResetting] = useState(false);
  const [ensureBusy, setEnsureBusy] = useState(false);
  const [ensureLog, setEnsureLog] = useState<string[]>([]);
  const [volumeCatalog, setVolumeCatalog] = useState<E2bVolumeCatalogEntry[]>([]);
  const [volumeCatalogLoading, setVolumeCatalogLoading] = useState(false);
  const [volumeMounts, setVolumeMounts] = useState<WorkerVolumeMountJson[]>([]);
  const mode = Form.useWatch("mode", form);
  const resourcesCustom = Form.useWatch("resourcesCustom", form);
  const cpuWatch = Form.useWatch("cpuCount", form);
  const memWatch = Form.useWatch("memoryMb", form);
  const harnessEngine = projectConfig?.harnessEngine;
  const engineStrictOnly = harnessEngine != null && harnessEngine !== "claw";
  const relaxedOffered = relaxedAllowed && !engineStrictOnly;

  const resourcesPreview =
    resourcesCustom &&
    typeof cpuWatch === "number" &&
    typeof memWatch === "number"
      ? normalizeWorkerResourcesClient(cpuWatch, memWatch)
      : null;

  const loadWorkerStatus = useCallback(async () => {
    setWorkerLoading(true);
    try {
      const r = await proxyHttp<ProjectE2bWorkerStatusResponse>(
        gatewayBase,
        "GET",
        `/v1/projects/${projId}/e2b-worker`
      );
      setWorkerStatus(r);
    } catch (e) {
      setWorkerStatus(null);
      message.error(e instanceof Error ? e.message : "加载 e2b worker 状态失败");
    } finally {
      setWorkerLoading(false);
    }
  }, [gatewayBase, projId]);

  const loadSystemDefault = useCallback(async () => {
    try {
      const r = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      setSystemDefault(r.strictLandlockDefault ?? null);
      setRelaxedAllowed(r.e2bPlatform?.relaxedWorkerAllowed !== false);
      setPoolSizeCap(r.e2bWorker?.poolSizeCap ?? 16);
      setGlobalPoolSize(r.e2bWorker?.poolSize ?? 1);
    } catch {
      setSystemDefault(null);
    }
  }, [gatewayBase]);

  const loadVolumeCatalog = useCallback(async () => {
    setVolumeCatalogLoading(true);
    try {
      const r = await proxyHttp<{ volumes: E2bVolumeCatalogEntry[] }>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings/e2b-volumes"
      );
      setVolumeCatalog(Array.isArray(r.volumes) ? r.volumes : []);
    } catch (e) {
      setVolumeCatalog([]);
      message.error(e instanceof Error ? e.message : "加载 e2b volumes 失败");
    } finally {
      setVolumeCatalogLoading(false);
    }
  }, [gatewayBase]);

  useEffect(() => {
    void loadSystemDefault();
  }, [loadSystemDefault]);

  useEffect(() => {
    void loadWorkerStatus();
  }, [loadWorkerStatus]);

  useEffect(() => {
    void loadVolumeCatalog();
  }, [loadVolumeCatalog]);

  useEffect(() => {
    const wp = projectConfig?.workerProfileJson;
    const rawMode =
      wp?.mode === "relaxed" && relaxedAllowed ? "relaxed" : "strict";
    const strict = wp?.strict;
    const inherit =
      rawMode === "strict" &&
      (strict?.useSystemDefault === true || !strict?.landlock);
    const landlock = strict?.landlock;
    const hasResources =
      wp?.resources != null &&
      typeof wp.resources.cpuCount === "number" &&
      typeof wp.resources.memoryMb === "number";
    form.setFieldsValue({
      mode: rawMode,
      poolSize: typeof wp?.poolSize === "number" ? wp.poolSize : null,
      askUserQuestionInAgent: wp?.askUserQuestionInAgent === true,
      resourcesCustom: hasResources,
      cpuCount: hasResources ? wp!.resources!.cpuCount : null,
      memoryMb: hasResources ? wp!.resources!.memoryMb : null,
      landlockInherit: inherit,
      landlockEnabled: landlock?.enabled ?? systemDefault?.enabled ?? true,
      landlockRw: landlock?.rw ?? systemDefault?.rw ?? ["${session_root}"],
      landlockRo: landlock?.ro ?? systemDefault?.ro ?? [],
    });
    const mounts = Array.isArray(wp?.volumeMounts)
      ? wp!.volumeMounts!
          .filter(
            (m) =>
              typeof m?.name === "string" &&
              m.name.trim() &&
              typeof m?.path === "string" &&
              m.path.trim()
          )
          .map((m) => ({ name: m.name.trim(), path: m.path.trim() }))
      : [];
    setVolumeMounts(mounts);
  }, [projectConfig, form, systemDefault, relaxedAllowed]);

  const fillFromSystemDefault = () => {
    if (!systemDefault) return;
    form.setFieldsValue({
      landlockEnabled: systemDefault.enabled,
      landlockRw: [...systemDefault.rw],
      landlockRo: [...systemDefault.ro],
    });
  };

  const buildWorkerProfileJson = (v: FormValues): WorkerProfileJson => {
    const askUser =
      v.askUserQuestionInAgent === true
        ? { askUserQuestionInAgent: true as const }
        : {};
    const existingRt = projectConfig?.workerProfileJson?.resourceTemplate;

    let resourcesBlock: Pick<WorkerProfileJson, "resources" | "resourceTemplate"> =
      {};
    if (v.resourcesCustom) {
      if (typeof v.cpuCount !== "number" || typeof v.memoryMb !== "number") {
        throw new Error("自定义资源时必须填写 cpu 与 memoryMb");
      }
      const norm = normalizeWorkerResourcesClient(v.cpuCount, v.memoryMb);
      if ("error" in norm) {
        throw new Error(norm.error);
      }
      const fp = workerResourcesFingerprint(norm);
      resourcesBlock = { resources: norm };
      if (
        existingRt &&
        existingRt.fingerprint === fp &&
        existingRt.templateId?.trim()
      ) {
        resourcesBlock.resourceTemplate = {
          templateId: existingRt.templateId,
          buildId: existingRt.buildId ?? undefined,
          fingerprint: fp,
        };
      }
    }
    // resourcesCustom=false → omit resources + resourceTemplate (reverse: clear pointer)

    const cleanedMounts = volumeMounts
      .map((m) => ({ name: m.name.trim(), path: m.path.trim() }))
      .filter((m) => m.name && m.path);
    for (const m of cleanedMounts) {
      if (!m.path.startsWith("/")) {
        throw new Error(`volumeMounts path 须为绝对路径：${m.name}`);
      }
      if (m.path.includes("..")) {
        throw new Error(`volumeMounts path 不能包含 ..：${m.name}`);
      }
    }
    const volumeBlock: Pick<WorkerProfileJson, "volumeMounts"> =
      cleanedMounts.length > 0 ? { volumeMounts: cleanedMounts } : {};

    if (v.mode === "relaxed") {
      return { mode: "relaxed", ...askUser, ...resourcesBlock, ...volumeBlock };
    }
    const poolSize =
      typeof v.poolSize === "number" && Number.isFinite(v.poolSize) ? v.poolSize : undefined;
    if (v.landlockInherit) {
      return {
        mode: "strict",
        ...(poolSize != null ? { poolSize } : {}),
        ...askUser,
        ...resourcesBlock,
        ...volumeBlock,
        strict: { useSystemDefault: true },
      };
    }
    const landlock: LandlockDsl = {
      enabled: v.landlockEnabled,
      rw: v.landlockRw.map((p) => p.trim()).filter(Boolean),
      ro: v.landlockRo.map((p) => p.trim()).filter(Boolean),
    };
    return {
      mode: "strict",
      ...(poolSize != null ? { poolSize } : {}),
      ...askUser,
      ...resourcesBlock,
      ...volumeBlock,
      strict: { useSystemDefault: false, landlock },
    };
  };

  const ensureResourceTemplate = async () => {
    if (!projectConfig) return;
    setEnsureBusy(true);
    setEnsureLog([]);
    try {
      // Persist current form resources first so Gateway reads the intended fingerprint.
      const v = await form.validateFields();
      const workerProfileJson = buildWorkerProfileJson(v);
      await putProjectConfigDraft(gatewayBase, projId, projectConfig, { workerProfileJson });
      await refreshProjectConfig();

      const start = await proxyHttp<{
        accepted: boolean;
        message?: string;
        job: {
          phase: string;
          message?: string;
          logTail?: string[];
          fingerprint?: string;
        };
      }>(gatewayBase, "POST", `/v1/projects/${projId}/e2b-worker/ensure-resource-template`);
      if (start.message) message.info(start.message);
      if (start.job.logTail?.length) setEnsureLog(start.job.logTail);

      if (!start.accepted && start.job.phase !== "running") {
        await refreshProjectConfig();
        return;
      }

      for (let i = 0; i < 120; i++) {
        await new Promise((r) => setTimeout(r, 3000));
        const st = await proxyHttp<{
          job: {
            phase: string;
            message?: string;
            logTail?: string[];
          };
        }>(gatewayBase, "GET", `/v1/projects/${projId}/e2b-worker/ensure-resource-template`);
        if (st.job.logTail?.length) setEnsureLog(st.job.logTail);
        if (st.job.phase === "succeeded") {
          message.success(st.job.message || "派生模板已绑定；请强制重建 worker");
          await refreshProjectConfig();
          await loadWorkerStatus();
          return;
        }
        if (st.job.phase === "failed") {
          throw new Error(st.job.message || "ensure 失败");
        }
      }
      throw new Error("ensure 超时（仍在 running）；请稍后刷新查看绑定状态");
    } catch (e) {
      message.error(e instanceof Error ? e.message : "ensure 派生模板失败");
    } finally {
      setEnsureBusy(false);
    }
  };

  const resetWorker = async (slotIndex?: number) => {
    setWorkerResetting(true);
    try {
      const path =
        slotIndex != null
          ? `/v1/projects/${projId}/e2b-worker/reset?slotIndex=${slotIndex}`
          : `/v1/projects/${projId}/e2b-worker/reset`;
      const r = await proxyHttp<ProjectE2bWorkerResetResponse>(
        gatewayBase,
        "POST",
        path
      );
      setWorkerStatus((prev) =>
        prev
          ? {
              ...prev,
              workers: r.workers,
              rotationLog: r.rotationLog,
            }
          : null
      );
      const label = slotIndex != null ? `slot ${slotIndex}` : "全部 slot";
      message.success(`Worker 已重建（${label}）`);
      await loadWorkerStatus();
    } catch (e) {
      message.error(e instanceof Error ? e.message : "重建 Worker 失败");
    } finally {
      setWorkerResetting(false);
    }
  };

  const isStrict = workerStatus?.workerProfile === "strict";

  return (
    <Card title="Worker 执行环境" size="small">
      <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
        存于 <Typography.Text code>project_config.worker_profile_json</Typography.Text>
        （项目 {projId}）。strict = solve worker 池；relaxed = 单 worker（可写容器）。
      </Typography.Paragraph>
      <Alert
        type="info"
        showIcon
        style={{ marginBottom: 16 }}
        message="e2b 基座（系统默认）"
        description={
          <>
            NAS 挂载 <Typography.Text code>/claw_ds</Typography.Text>、
            <Typography.Text code>/claw_host_root</Typography.Text> 或{" "}
            <Typography.Text code>/claw_sessions</Typography.Text>。
          </>
        }
      />
      <Card
        type="inner"
        title={
          isStrict
            ? `e2b Worker 池（strict · 目标 ${workerStatus?.desiredPoolSize ?? globalPoolSize}）`
            : "e2b Worker（relaxed · 单实例）"
        }
        size="small"
        style={{ marginBottom: 16 }}
        extra={
          <Space>
            <Button
              size="small"
              icon={<ReloadOutlined />}
              loading={workerLoading}
              onClick={() => void loadWorkerStatus()}
            >
              刷新
            </Button>
            {isStrict ? (
              <Popconfirm
                title="强制重建全部 Worker slot？"
                description="将 kill 当前项目全部 warm worker 并按最新模板重新创建。"
                okText="重建全部"
                cancelText="取消"
                onConfirm={() => void resetWorker()}
              >
                <Button size="small" type="primary" danger loading={workerResetting}>
                  强制重建全部
                </Button>
              </Popconfirm>
            ) : (
              <Popconfirm
                title="强制重建 Worker？"
                description="将 kill 当前项目 warm worker 并按最新模板重新创建。"
                okText="重建"
                cancelText="取消"
                onConfirm={() => void resetWorker()}
              >
                <Button size="small" type="primary" danger loading={workerResetting}>
                  强制重建
                </Button>
              </Popconfirm>
            )}
          </Space>
        }
      >
        {workerLoading && !workerStatus ? (
          <Spin />
        ) : workerStatus && workerStatus.workers.length > 0 ? (
          <>
            <Descriptions size="small" column={1} bordered style={{ marginBottom: 12 }}>
              <Descriptions.Item label="期望模板">
                {workerStatus.desiredTemplate}
              </Descriptions.Item>
              {isStrict ? (
                <Descriptions.Item label="池大小">
                  {workerStatus.workers.length} / {workerStatus.desiredPoolSize} slot
                </Descriptions.Item>
              ) : null}
            </Descriptions>
            {isStrict ? (
              <Table
                size="small"
                pagination={false}
                rowKey={(row) => String(row.slotIndex)}
                dataSource={workerStatus.workers}
                columns={[
                  { title: "slot", dataIndex: "slotIndex", width: 56 },
                  {
                    title: "sandboxId",
                    dataIndex: "sandboxId",
                    ellipsis: true,
                    render: (v: string) => <Typography.Text copyable>{v}</Typography.Text>,
                  },
                  {
                    title: "workerId",
                    dataIndex: "workerId",
                    ellipsis: true,
                    render: (v: string) => <Typography.Text copyable>{v}</Typography.Text>,
                  },
                  {
                    title: "状态",
                    width: 100,
                    render: (_: unknown, row: ProjectE2bWorkerInfo) =>
                      row.running ? <Tag color="green">running</Tag> : <Tag color="red">offline</Tag>,
                  },
                  {
                    title: "lease",
                    dataIndex: "activeLeases",
                    width: 64,
                    render: (n?: number) => n ?? 0,
                  },
                  {
                    title: "操作",
                    width: 100,
                    render: (_: unknown, row: ProjectE2bWorkerInfo) => (
                      <Popconfirm
                        title={`重建 slot ${row.slotIndex}？`}
                        onConfirm={() => void resetWorker(row.slotIndex)}
                        okText="重建"
                        cancelText="取消"
                      >
                        <Button size="small" loading={workerResetting}>
                          重建
                        </Button>
                      </Popconfirm>
                    ),
                  },
                ]}
              />
            ) : (
              (() => {
                const w = workerStatus.workers[0];
                return (
                  <Descriptions size="small" column={1} bordered>
                    <Descriptions.Item label="sandboxId">
                      <Typography.Text copyable>{w.sandboxId}</Typography.Text>
                    </Descriptions.Item>
                    <Descriptions.Item label="workerId">
                      <Typography.Text copyable>{w.workerId}</Typography.Text>
                    </Descriptions.Item>
                    <Descriptions.Item label="模板契约">{w.templateContract}</Descriptions.Item>
                    <Descriptions.Item label="运行状态">
                      {w.running ? <Tag color="green">running</Tag> : <Tag color="red">offline</Tag>}
                      {w.remainingTtlSecs != null ? (
                        <Typography.Text type="secondary" style={{ marginLeft: 8 }}>
                          TTL {w.remainingTtlSecs}s
                        </Typography.Text>
                      ) : null}
                    </Descriptions.Item>
                    <Descriptions.Item label="e2b API">
                      <Typography.Text copyable>{w.urls.e2bApiUrl}</Typography.Text>
                    </Descriptions.Item>
                    {w.urls.trafficProxyBase ? (
                      <Descriptions.Item label="Traffic 代理">
                        <Typography.Text copyable>{w.urls.trafficProxyBase}</Typography.Text>
                      </Descriptions.Item>
                    ) : null}
                    <Descriptions.Item label="沙箱域名">
                      <Typography.Text copyable>{w.urls.sandboxDomain}</Typography.Text>
                    </Descriptions.Item>
                  </Descriptions>
                );
              })()
            )}
            {workerStatus.rotationLog.length > 0 ? (
              <Table
                size="small"
                style={{ marginTop: 12 }}
                pagination={false}
                rowKey={(row) => `${row.atMs}-${row.event}-${row.sandboxId ?? ""}`}
                dataSource={workerStatus.rotationLog}
                columns={[
                  { title: "事件", dataIndex: "event", width: 100 },
                  { title: "sandboxId", dataIndex: "sandboxId", ellipsis: true },
                  { title: "原因", dataIndex: "reason", ellipsis: true },
                  {
                    title: "时间",
                    dataIndex: "atMs",
                    width: 180,
                    render: (ms: number) => new Date(ms).toLocaleString(),
                  },
                ]}
              />
            ) : null}
          </>
        ) : (
          <Alert
            type="warning"
            showIcon
            message="当前项目尚无 warm worker"
            description={`期望模板：${workerStatus?.desiredTemplate ?? "claw-worker"}。可点击「强制重建」创建。`}
          />
        )}
      </Card>
      <Form form={form} layout="vertical">
        <Form.Item name="mode" label="Worker profile">
          <Radio.Group>
            <Radio value="strict">Strict（Landlock session 隔离）</Radio>
            {relaxedOffered ? (
              <Radio value="relaxed">Relaxed（root + 可写容器）</Radio>
            ) : null}
          </Radio.Group>
        </Form.Item>
        {engineStrictOnly ? (
          <Alert
            type="info"
            showIcon
            style={{ marginBottom: 16 }}
            message={`${harnessEngine} 引擎项目仅支持 Strict worker`}
            description="接口拒绝写入 mode=relaxed；Agent 提问（ask-user）对该引擎不生效。"
          />
        ) : null}
        <Form.Item
          name="askUserQuestionInAgent"
          label="Agent 模式允许向用户提问"
          valuePropName="checked"
          extra="Plan 模式始终可用 AskUserQuestion；开启后 Agent 模式也会注册该工具。"
        >
          <Switch />
        </Form.Item>
        {!relaxedAllowed ? (
          <Alert
            type="warning"
            showIcon
            style={{ marginBottom: 16 }}
            message="当前网关为严格模式（CLAW_ALLOW_RELAXED_WORKER=false）"
            description="已隐藏 Relaxed 选项；接口亦拒绝写入 mode=relaxed。"
          />
        ) : null}

        {mode === "strict" ? (
          <Form.Item
            name="poolSize"
            label="本项目 poolSize（可选）"
            extra={`留空则继承全局默认 ${globalPoolSize}；范围 1–${poolSizeCap}（CLAW_E2B_POOL_SIZE_CAP）。`}
            rules={[
              {
                type: "number",
                min: 1,
                max: poolSizeCap,
                message: `须在 1–${poolSizeCap} 之间`,
              },
            ]}
          >
            <InputNumber
              min={1}
              max={poolSizeCap}
              placeholder={`全局默认 ${globalPoolSize}`}
              style={{ width: 160 }}
            />
          </Form.Item>
        ) : null}

        <Card
          type="inner"
          title="Worker 资源（cpu / memory）"
          size="small"
          style={{ marginBottom: 16 }}
        >
          <Form.Item
            name="resourcesCustom"
            label="规格来源"
            extra="默认使用全局公用模板；自定义后在本页点「确保派生模板」再强制重建。"
          >
            <Radio.Group>
              <Radio value={false}>系统默认（全局模板）</Radio>
              <Radio value={true}>本项目自定义</Radio>
            </Radio.Group>
          </Form.Item>
          {resourcesCustom ? (
            <>
              <Space wrap style={{ marginBottom: 8 }}>
                <Form.Item
                  name="cpuCount"
                  label="CPU"
                  rules={[{ required: true, message: "必填" }]}
                  style={{ marginBottom: 0 }}
                >
                  <InputNumber
                    min={0.1}
                    max={WORKER_CPU_MAX}
                    step={1}
                    style={{ width: 120 }}
                  />
                </Form.Item>
                <Form.Item
                  name="memoryMb"
                  label="Memory (MiB)"
                  rules={[{ required: true, message: "必填" }]}
                  style={{ marginBottom: 0 }}
                >
                  <InputNumber
                    min={1}
                    max={WORKER_MEMORY_MAX_MB}
                    step={WORKER_MEMORY_QUANTUM_MB}
                    style={{ width: 160 }}
                  />
                </Form.Item>
              </Space>
              {resourcesPreview && "error" in resourcesPreview ? (
                <Alert type="error" showIcon message={resourcesPreview.error} />
              ) : resourcesPreview ? (
                <Alert
                  type="info"
                  showIcon
                  message={`规范化预览：cpu=${resourcesPreview.cpuCount} memoryMb=${resourcesPreview.memoryMb}（指纹 ${workerResourcesFingerprint(resourcesPreview)}）`}
                  description="任意值会向上取整：cpu 整数、memory 按 256 MiB。指纹与已绑定模板不一致时，点「确保派生模板」。"
                />
              ) : null}
              <Space wrap style={{ marginTop: 12 }}>
                <Button
                  type="primary"
                  loading={ensureBusy}
                  disabled={!resourcesPreview || "error" in (resourcesPreview || {})}
                  onClick={() => void ensureResourceTemplate()}
                >
                  确保派生模板
                </Button>
                <Typography.Text type="secondary">
                  Gateway 打 e2b SKU 模板并写回 resourceTemplate；完成后点上方「强制重建」。
                </Typography.Text>
              </Space>
              {ensureLog.length > 0 ? (
                <Typography.Paragraph
                  code
                  style={{
                    marginTop: 12,
                    maxHeight: 160,
                    overflow: "auto",
                    whiteSpace: "pre-wrap",
                    fontSize: 12,
                  }}
                >
                  {ensureLog.slice(-40).join("\n")}
                </Typography.Paragraph>
              ) : null}
              {projectConfig?.workerProfileJson?.resourceTemplate ? (
                <>
                  <Descriptions size="small" column={1} style={{ marginTop: 12 }} bordered>
                    <Descriptions.Item label="已绑定 templateId">
                      <Typography.Text copyable>
                        {projectConfig.workerProfileJson.resourceTemplate.templateId}
                      </Typography.Text>
                    </Descriptions.Item>
                    <Descriptions.Item label="buildId（起沙 pin）">
                      <Typography.Text copyable>
                        {projectConfig.workerProfileJson.resourceTemplate.buildId || "—"}
                      </Typography.Text>
                    </Descriptions.Item>
                    <Descriptions.Item label="fingerprint">
                      {projectConfig.workerProfileJson.resourceTemplate.fingerprint}
                    </Descriptions.Item>
                  </Descriptions>
                  <Alert
                    type="info"
                    showIcon
                    style={{ marginTop: 12 }}
                    message="验收口径：强制重建后以 e2b/cgroup 为准"
                    description="本页 cpu/memory 经「确保派生模板」写入 build；起沙 templateID=buildId。内存必须受 cgroup memory.max 约束（超限 OOM）。容器内 lscpu、/proc/meminfo MemTotal 常仍显示宿主机，不代表未限流——CPU 看 cpu.max/NanoCpus，内存看 memory.max。"
                  />
                </>
              ) : (
                <Alert
                  type="warning"
                  showIcon
                  style={{ marginTop: 12 }}
                  message="尚未绑定 resourceTemplate"
                  description="保存自定义规格后，点「确保派生模板」；成功后再「强制重建」。"
                />
              )}
            </>
          ) : (
            <Alert
              type="success"
              showIcon
              message="使用全局 e2b worker 模板"
              description="关闭自定义会清空 resources 与 resourceTemplate，回落到全局模板。"
            />
          )}
        </Card>

        <Card
          type="inner"
          title="e2b Volume 挂载（volumeMounts）"
          size="small"
          style={{ marginBottom: 16 }}
          extra={
            <Button
              size="small"
              icon={<ReloadOutlined />}
              loading={volumeCatalogLoading}
              onClick={() => void loadVolumeCatalog()}
            >
              刷新清单
            </Button>
          }
        >
          <Alert
            type="info"
            showIcon
            style={{ marginBottom: 12 }}
            message="清单来自 e2b GET /volumes；物理后端（含 CIFS 账密）只在 e2b Admin 绑定"
            description="与内部 nasConfig 工程盘并存。保存后需「强制重建」worker 才会挂进沙箱。容器内用 path（如 /mnt/ivqa）直接读写，勿再 smbclient。"
          />
          {volumeMounts.map((row, idx) => (
            <Space
              key={`${row.name}-${idx}`}
              align="start"
              wrap
              style={{ display: "flex", marginBottom: 8 }}
            >
              <Select
                style={{ width: 220 }}
                placeholder="逻辑卷 name"
                value={row.name || undefined}
                options={volumeCatalog.map((v) => ({
                  value: v.name,
                  label: v.name,
                }))}
                showSearch
                optionFilterProp="label"
                onChange={(name: string) => {
                  setVolumeMounts((prev) =>
                    prev.map((m, i) =>
                      i === idx
                        ? {
                            name,
                            path: m.path?.startsWith("/")
                              ? m.path
                              : `/mnt/${name}`,
                          }
                        : m
                    )
                  );
                }}
              />
              <Input
                style={{ width: 240 }}
                placeholder="容器内 path，如 /mnt/ivqa"
                value={row.path}
                onChange={(e) => {
                  const path = e.target.value;
                  setVolumeMounts((prev) =>
                    prev.map((m, i) => (i === idx ? { ...m, path } : m))
                  );
                }}
              />
              <MinusCircleOutlined
                onClick={() =>
                  setVolumeMounts((prev) => prev.filter((_, i) => i !== idx))
                }
              />
            </Space>
          ))}
          <Button
            type="dashed"
            size="small"
            icon={<PlusOutlined />}
            disabled={volumeCatalog.length === 0}
            onClick={() => {
              const first = volumeCatalog[0];
              if (!first) return;
              setVolumeMounts((prev) => [
                ...prev,
                { name: first.name, path: `/mnt/${first.name}` },
              ]);
            }}
          >
            添加挂载
          </Button>
          {volumeCatalog.length === 0 && !volumeCatalogLoading ? (
            <Typography.Text type="secondary" style={{ display: "block", marginTop: 8 }}>
              e2b 尚无逻辑卷；请先在 e2b Admin → Volume 创建并绑定后端。
            </Typography.Text>
          ) : null}
        </Card>

        {mode === "strict" ? (
          <Card type="inner" title="Landlock 隔离规则" size="small" style={{ marginBottom: 16 }}>
            <Form.Item name="landlockInherit" label="配置来源">
              <Radio.Group>
                <Radio value={true}>
                  继承系统预制默认{" "}
                  <Tag color="blue">系统默认</Tag>
                </Radio>
                <Radio value={false}>
                  项目自定义 <Tag color="orange">项目</Tag>
                </Radio>
              </Radio.Group>
            </Form.Item>

            <Form.Item noStyle shouldUpdate={(prev, cur) => prev.landlockInherit !== cur.landlockInherit}>
              {({ getFieldValue }) =>
                getFieldValue("landlockInherit") ? (
                  <Alert
                    type="success"
                    showIcon
                    style={{ marginBottom: 12 }}
                    message="当前使用系统 strictLandlockDefault"
                    description="可在「全局配置 → Strict Landlock」修改系统默认。"
                  />
                ) : (
                  <>
                    <Space style={{ marginBottom: 12 }}>
                      <Button size="small" onClick={fillFromSystemDefault} disabled={!systemDefault}>
                        从系统默认填充
                      </Button>
                    </Space>
                    <Form.Item name="landlockEnabled" label="启用 Landlock" valuePropName="checked">
                      <Switch />
                    </Form.Item>
                    {(["landlockRw", "landlockRo"] as const).map((fieldName, idx) => {
                      const kind = idx === 0 ? "rw" : "ro";
                      return (
                        <Form.List key={fieldName} name={fieldName}>
                          {(fields, { add, remove }) => (
                            <div style={{ marginBottom: 12 }}>
                              <Typography.Text strong>{kind.toUpperCase()} 路径</Typography.Text>
                              {fields.map((field) => (
                                <Space key={field.key} align="baseline" style={{ display: "flex", marginTop: 8 }}>
                                  <Form.Item
                                    {...field}
                                    rules={[{ required: true, message: "必填" }]}
                                    style={{ flex: 1, marginBottom: 0 }}
                                  >
                                    <Input />
                                  </Form.Item>
                                  <MinusCircleOutlined onClick={() => remove(field.name)} />
                                </Space>
                              ))}
                              <Button
                                type="dashed"
                                size="small"
                                onClick={() => add("")}
                                icon={<PlusOutlined />}
                                style={{ marginTop: 8 }}
                              >
                                添加 {kind}
                              </Button>
                            </div>
                          )}
                        </Form.List>
                      );
                    })}
                  </>
                )
              }
            </Form.Item>
          </Card>
        ) : null}
      </Form>
      <Space>
        <Button
          type="primary"
          onClick={async () => {
            if (!projectConfig) return;
            const v = await form.validateFields();
            let workerProfileJson: WorkerProfileJson;
            try {
              workerProfileJson = buildWorkerProfileJson(v);
            } catch (e) {
              message.error(e instanceof Error ? e.message : String(e));
              return;
            }
            if (
              v.mode === "strict" &&
              !v.landlockInherit &&
              workerProfileJson.strict?.landlock
            ) {
              const err = validateLandlockDslClient(workerProfileJson.strict.landlock);
              if (err) {
                message.error(err);
                return;
              }
            }
            await putProjectConfigDraft(gatewayBase, projId, projectConfig, {
              workerProfileJson,
            });
            const cleared = !v.resourcesCustom;
            message.success(
              cleared
                ? "已保存：resources 已清空，将回落全局模板；请强制重建 worker"
                : workerProfileJson.resourceTemplate
                  ? "Worker profile 已保存；请强制重建 worker 使规格生效"
                  : "resources 已保存；请点「确保派生模板」，再强制重建"
            );
            await refreshProjectConfig();
            await loadWorkerStatus();
          }}
        >
          保存 Worker profile
        </Button>
      </Space>
    </Card>
  );
}
