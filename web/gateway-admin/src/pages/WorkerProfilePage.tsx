import {
  Alert,
  Button,
  Card,
  Form,
  Input,
  Radio,
  Space,
  Switch,
  Tag,
  Typography,
  message,
} from "antd";
import { MinusCircleOutlined, PlusOutlined } from "@ant-design/icons";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import { putProjectConfigDraft } from "../utils/projectConfig";
import type { GlobalSettingsResponse } from "../types/globalSettings";
import type { LandlockDsl, WorkerProfileJson } from "../types/landlock";
import { validateLandlockDslClient } from "../types/landlock";

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
  const mode = Form.useWatch("mode", form);

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
