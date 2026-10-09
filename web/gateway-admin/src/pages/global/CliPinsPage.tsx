import { CloudDownloadOutlined, ReloadOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Form, Input, Space, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type { CliPins, GlobalSettingsResponse } from "../../types/globalSettings";

type PinField = keyof CliPins;

const FIELDS: { key: PinField; label: string; hint: string }[] = [
  { key: "claw", label: "claw", hint: "默认 solve CLI（必填才能起 worker）" },
  { key: "neuroOpencode", label: "neuro-opencode", hint: "协议适配层（opencode 项目）" },
  { key: "neuroAppserver", label: "neuro-appserver", hint: "协议适配层（appserver 项目）" },
  { key: "acpOpencode", label: "acp-opencode", hint: "opencode 引擎包（可独立升）" },
  { key: "acpAppserver", label: "acp-appserver", hint: "codex-acp 引擎包（可独立升）" },
];

/** Admin: apply Worker CLI version pins (full registry ref). Author: kejiqing */
export default function CliPinsPage() {
  const { gatewayBase } = useApp();
  const [form] = Form.useForm<Record<PinField, string>>();
  const [digests, setDigests] = useState<Partial<Record<PinField, string>>>({});
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      const pins = r.cliPins ?? {};
      const values: Partial<Record<PinField, string>> = {};
      const dig: Partial<Record<PinField, string>> = {};
      for (const f of FIELDS) {
        values[f.key] = pins[f.key]?.ref ?? "";
        dig[f.key] = pins[f.key]?.digest ?? "";
      }
      form.setFieldsValue(values as Record<PinField, string>);
      setDigests(dig);
    } finally {
      setLoading(false);
    }
  }, [gatewayBase, form]);

  useEffect(() => {
    void load();
  }, [load]);

  const onApply = async () => {
    const values = await form.validateFields();
    const body: CliPins = {};
    for (const f of FIELDS) {
      const ref = (values[f.key] ?? "").trim();
      if (ref) {
        body[f.key] = { ref };
      }
    }
    setSaving(true);
    try {
      const r = await proxyHttp<CliPins>(
        gatewayBase,
        "PUT",
        "/v1/gateway/global-settings/cli-pins",
        body
      );
      message.success("已应用 CLI 版本（健康沙箱不会自动重建，请按需重置 worker）");
      const dig: Partial<Record<PinField, string>> = {};
      for (const f of FIELDS) {
        dig[f.key] = r[f.key]?.digest ?? "";
        form.setFieldValue(f.key, r[f.key]?.ref ?? "");
      }
      setDigests(dig);
    } catch (e) {
      message.error(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Space direction="vertical" size="large" style={{ width: "100%" }}>
      <Space style={{ width: "100%", justifyContent: "space-between" }}>
        <Typography.Title level={4} style={{ margin: 0 }}>
          <CloudDownloadOutlined /> Worker CLI 版本
        </Typography.Title>
        <Button icon={<ReloadOutlined />} loading={loading} onClick={() => void load()}>
          刷新
        </Button>
      </Space>

      <Alert
        type="info"
        showIcon
        message="钉死版本（pin）"
        description={
          <Typography.Paragraph style={{ marginBottom: 0 }}>
            先用 <Typography.Text code>deploy/pack/publish.sh cli-*</Typography.Text> 把制品推进
            Nora/ACR，再在此填<strong>完整 ref</strong>（含 registry 前缀）。Gateway 校验并写入
            digest。新沙箱 create 时由平台 worker.init 安装；已有健康沙箱需手动重置 worker。
          </Typography.Paragraph>
        }
      />

      <Card size="small">
        <Form form={form} layout="vertical">
          {FIELDS.map((f) => (
            <Form.Item
              key={f.key}
              name={f.key}
              label={f.label}
              extra={
                <>
                  {f.hint}
                  {digests[f.key] ? (
                    <>
                      {" "}
                      · digest <Typography.Text code>{digests[f.key]}</Typography.Text>
                    </>
                  ) : null}
                </>
              }
            >
              <Input placeholder="host/ns/claw-cli/claw:v1.2.3" allowClear />
            </Form.Item>
          ))}
          <Button type="primary" loading={saving} onClick={() => void onApply()}>
            应用
          </Button>
        </Form>
      </Card>
    </Space>
  );
}
