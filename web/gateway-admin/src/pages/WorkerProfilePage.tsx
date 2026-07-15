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
import type { LandlockDsl, WorkerProfileJson } from "../types/landlock";
import { validateLandlockDslClient } from "../types/landlock";
import type {
  ProjectE2bWorkerInfo,
  ProjectE2bWorkerResetResponse,
  ProjectE2bWorkerStatusResponse,
} from "../types/projectE2bWorker";

type Mode = WorkerProfileJson["mode"];

type FormValues = {
  mode: Mode;
  poolSize?: number | null;
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
  const mode = Form.useWatch("mode", form);

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

  useEffect(() => {
    void loadSystemDefault();
  }, [loadSystemDefault]);

  useEffect(() => {
    void loadWorkerStatus();
  }, [loadWorkerStatus]);

  useEffect(() => {
    const wp = projectConfig?.workerProfileJson;
    const rawMode =
      wp?.mode === "relaxed" && relaxedAllowed ? "relaxed" : "strict";
    const strict = wp?.strict;
    const inherit =
      rawMode === "strict" &&
      (strict?.useSystemDefault === true || !strict?.landlock);
    const landlock = strict?.landlock;
    form.setFieldsValue({
      mode: rawMode,
      poolSize: typeof wp?.poolSize === "number" ? wp.poolSize : null,
      landlockInherit: inherit,
      landlockEnabled: landlock?.enabled ?? systemDefault?.enabled ?? true,
      landlockRw: landlock?.rw ?? systemDefault?.rw ?? ["${session_root}"],
      landlockRo: landlock?.ro ?? systemDefault?.ro ?? [],
    });
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
    if (v.mode === "relaxed") {
      return { mode: "relaxed" };
    }
    const poolSize =
      typeof v.poolSize === "number" && Number.isFinite(v.poolSize) ? v.poolSize : undefined;
    if (v.landlockInherit) {
      return {
        mode: "strict",
        ...(poolSize != null ? { poolSize } : {}),
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
      strict: { useSystemDefault: false, landlock },
    };
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
        （项目 {projId}）。strict = solve worker 池（无 ttyd）；relaxed = 单 worker + OVS/ttyd 交互。
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
            : "e2b Worker（relaxed · 单实例 + ttyd）"
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
                    {w.urls.ttydPublicHost ? (
                      <Descriptions.Item label="ttyd Host">
                        <Typography.Text copyable>{w.urls.ttydPublicHost}</Typography.Text>
                      </Descriptions.Item>
                    ) : null}
                    {w.urls.ttydWsUrl ? (
                      <Descriptions.Item label="ttyd WS URL">
                        <Typography.Text copyable style={{ wordBreak: "break-all" }}>
                          {w.urls.ttydWsUrl}
                        </Typography.Text>
                      </Descriptions.Item>
                    ) : null}
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
            {relaxedAllowed ? (
              <Radio value="relaxed">Relaxed（OVS：root + 可写容器）</Radio>
            ) : null}
          </Radio.Group>
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
            const workerProfileJson = buildWorkerProfileJson(v);
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
            message.success("Worker profile 已保存；下次 solve 时生效");
            await refreshProjectConfig();
          }}
        >
          保存 Worker profile
        </Button>
      </Space>
    </Card>
  );
}
