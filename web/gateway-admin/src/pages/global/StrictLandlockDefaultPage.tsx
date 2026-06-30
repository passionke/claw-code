import { LockOutlined, ReloadOutlined } from "@ant-design/icons";
import {
  Alert,
  Button,
  Card,
  Form,
  Input,
  Space,
  Switch,
  Typography,
  message,
} from "antd";
import { MinusCircleOutlined, PlusOutlined } from "@ant-design/icons";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type { GlobalSettingsResponse } from "../../types/globalSettings";
import type { LandlockDsl } from "../../types/landlock";
import { validateLandlockDslClient } from "../../types/landlock";

/** System-wide strict Landlock default DSL. Author: kejiqing */
export default function StrictLandlockDefaultPage() {
  const { gatewayBase } = useApp();
  const [form] = Form.useForm<LandlockDsl>();
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      const dsl = r.strictLandlockDefault ?? {
        enabled: true,
        rw: ["${session_root}"],
        ro: ["/usr"],
      };
      form.setFieldsValue(dsl);
    } finally {
      setLoading(false);
    }
  }, [gatewayBase, form]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <Space direction="vertical" size="large" style={{ width: "100%" }}>
      <Space style={{ width: "100%", justifyContent: "space-between" }}>
        <Typography.Title level={4} style={{ margin: 0 }}>
          <LockOutlined /> Strict Landlock 预制默认
        </Typography.Title>
        <Button icon={<ReloadOutlined />} loading={loading} onClick={() => void load()}>
          刷新
        </Button>
      </Space>

      <Alert
        type="info"
        showIcon
        message="系统级默认"
        description={
          <Typography.Paragraph style={{ marginBottom: 0 }}>
            存于 <Typography.Text code>gateway_global_settings.strictLandlockDefault</Typography.Text>
            。strict 项目未自定义时继承此配置；修改后无需重新发布 worker。
          </Typography.Paragraph>
        }
      />

      <Card size="small">
        <Form form={form} layout="vertical" initialValues={{ enabled: true, rw: [], ro: [] }}>
          <Form.Item name="enabled" label="启用 Landlock" valuePropName="checked">
            <Switch />
          </Form.Item>
          <Typography.Text type="secondary">
            变量：<Typography.Text code>${"{"}session_root{"}"}</Typography.Text>、
            <Typography.Text code>${"{"}project_home_def{"}"}</Typography.Text>、
            <Typography.Text code>${"{"}tmpdir{"}"}</Typography.Text>、
            <Typography.Text code>${"{"}claw_bin_dir{"}"}</Typography.Text>
          </Typography.Text>

          {(["rw", "ro"] as const).map((kind) => (
            <Form.List key={kind} name={kind}>
              {(fields, { add, remove }) => (
                <div style={{ marginTop: 16 }}>
                  <Typography.Text strong>{kind.toUpperCase()} 路径</Typography.Text>
                  {fields.map((field) => (
                    <Space key={field.key} align="baseline" style={{ display: "flex", marginTop: 8 }}>
                      <Form.Item {...field} rules={[{ required: true, message: "必填" }]} style={{ flex: 1, marginBottom: 0 }}>
                        <Input placeholder={kind === "rw" ? "${session_root}/work" : "/usr"} />
                      </Form.Item>
                      <MinusCircleOutlined onClick={() => remove(field.name)} />
                    </Space>
                  ))}
                  <Button type="dashed" onClick={() => add("")} icon={<PlusOutlined />} style={{ marginTop: 8 }}>
                    添加 {kind} 路径
                  </Button>
                </div>
              )}
            </Form.List>
          ))}
        </Form>

        <Space style={{ marginTop: 16 }}>
          <Button
            type="primary"
            loading={loading}
            onClick={async () => {
              const dsl = await form.validateFields();
              const err = validateLandlockDslClient(dsl);
              if (err) {
                message.error(err);
                return;
              }
              setLoading(true);
              try {
                await proxyHttp(gatewayBase, "PUT", "/v1/gateway/global-settings/strict-landlock-default", dsl);
                message.success("系统 Landlock 默认已保存");
                await load();
              } finally {
                setLoading(false);
              }
            }}
          >
            保存系统默认
          </Button>
        </Space>
      </Card>
    </Space>
  );
}
