import {
  Alert,
  Button,
  Card,
  Descriptions,
  Form,
  Input,
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
  ProjectE2bWorkerResetResponse,
  ProjectE2bWorkerStatusResponse,
} from "../types/projectE2bWorker";

type Mode = WorkerProfileJson["mode"];

type FormValues = {
  mode: Mode;
  landlockInherit: boolean;
  landlockEnabled: boolean;
  landlockRw: string[];
  landlockRo: string[];
};

export default function WorkerProfilePage() {
  const { gatewayBase, projId, projectConfig, refreshProjectConfig } = useApp();
  const [form] = Form.useForm<FormValues>();
  const [systemDefault, setSystemDefault] = useState<LandlockDsl | null>(null);
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
    const rawMode = wp?.mode === "relaxed" ? "relaxed" : "strict";
    const strict = wp?.strict;
    const inherit =
      rawMode === "strict" &&
      (strict?.useSystemDefault === true || !strict?.landlock);
    const landlock = strict?.landlock;
    form.setFieldsValue({
      mode: rawMode,
      landlockInherit: inherit,
      landlockEnabled: landlock?.enabled ?? systemDefault?.enabled ?? true,
      landlockRw: landlock?.rw ?? systemDefault?.rw ?? ["${session_root}"],
      landlockRo: landlock?.ro ?? systemDefault?.ro ?? [],
    });
  }, [projectConfig, form, systemDefault]);

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
    if (v.landlockInherit) {
      return { mode: "strict", strict: { useSystemDefault: true } };
    }
    const landlock: LandlockDsl = {
      enabled: v.landlockEnabled,
      rw: v.landlockRw.map((p) => p.trim()).filter(Boolean),
      ro: v.landlockRo.map((p) => p.trim()).filter(Boolean),
    };
    return {
      mode: "strict",
      strict: { useSystemDefault: false, landlock },
    };
  };

  return (
    <Card title="Worker 执行环境" size="small">
      <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
        存于 <Typography.Text code>project_config.worker_profile_json</Typography.Text>
        （项目 {projId}）。Worker 在 e2b 沙箱运行；strict 模式通过 Landlock per-solve 做 session 隔离。
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
        title="e2b Worker 沙箱"
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
            <Popconfirm
              title="强制重建 Worker？"
              description="将 kill 当前项目 warm worker 并按最新模板重新创建。"
              okText="重建"
              cancelText="取消"
              onConfirm={async () => {
                setWorkerResetting(true);
                try {
                  const r = await proxyHttp<ProjectE2bWorkerResetResponse>(
                    gatewayBase,
                    "POST",
                    `/v1/projects/${projId}/e2b-worker/reset`
                  );
                  setWorkerStatus({
                    projId: r.projId,
                    desiredTemplate: workerStatus?.desiredTemplate ?? "claw-worker",
                    worker: r.worker,
                    rotationLog: r.rotationLog,
                  });
                  message.success(`Worker 已重建：${r.worker.sandboxId}`);
                  await loadWorkerStatus();
                } catch (e) {
                  message.error(e instanceof Error ? e.message : "重建 Worker 失败");
                } finally {
                  setWorkerResetting(false);
                }
              }}
            >
              <Button size="small" type="primary" danger loading={workerResetting}>
                强制重建最新 Worker
              </Button>
            </Popconfirm>
          </Space>
        }
      >
        {workerLoading && !workerStatus ? (
          <Spin />
        ) : workerStatus?.worker ? (
          <>
            <Descriptions size="small" column={1} bordered>
              <Descriptions.Item label="sandboxId">
                <Typography.Text copyable>{workerStatus.worker.sandboxId}</Typography.Text>
              </Descriptions.Item>
              <Descriptions.Item label="workerId">
                <Typography.Text copyable>{workerStatus.worker.workerId}</Typography.Text>
              </Descriptions.Item>
              <Descriptions.Item label="模板契约">
                {workerStatus.worker.templateContract}
              </Descriptions.Item>
              <Descriptions.Item label="期望模板">
                {workerStatus.desiredTemplate}
              </Descriptions.Item>
              <Descriptions.Item label="运行状态">
                {workerStatus.worker.running ? (
                  <Tag color="green">running</Tag>
                ) : (
                  <Tag color="red">offline</Tag>
                )}
                {workerStatus.worker.remainingTtlSecs != null ? (
                  <Typography.Text type="secondary" style={{ marginLeft: 8 }}>
                    TTL {workerStatus.worker.remainingTtlSecs}s
                  </Typography.Text>
                ) : null}
              </Descriptions.Item>
              <Descriptions.Item label="e2b API">
                <Typography.Text copyable>{workerStatus.worker.urls.e2bApiUrl}</Typography.Text>
              </Descriptions.Item>
              {workerStatus.worker.urls.trafficProxyBase ? (
                <Descriptions.Item label="Traffic 代理">
                  <Typography.Text copyable>
                    {workerStatus.worker.urls.trafficProxyBase}
                  </Typography.Text>
                </Descriptions.Item>
              ) : null}
              <Descriptions.Item label="沙箱域名">
                <Typography.Text copyable>{workerStatus.worker.urls.sandboxDomain}</Typography.Text>
              </Descriptions.Item>
              <Descriptions.Item label="ttyd Host">
                <Typography.Text copyable>{workerStatus.worker.urls.ttydPublicHost}</Typography.Text>
              </Descriptions.Item>
              <Descriptions.Item label="ttyd WS URL">
                <Typography.Text copyable style={{ wordBreak: "break-all" }}>
                  {workerStatus.worker.urls.ttydWsUrl}
                </Typography.Text>
              </Descriptions.Item>
            </Descriptions>
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
            <Radio value="relaxed">Relaxed（OVS：root + 可写容器）</Radio>
          </Radio.Group>
        </Form.Item>

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
