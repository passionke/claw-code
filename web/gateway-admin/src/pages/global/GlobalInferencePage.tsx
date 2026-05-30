import { ApiOutlined, SaveOutlined } from "@ant-design/icons";
import {
  Alert,
  Button,
  Card,
  Divider,
  Form,
  Input,
  InputNumber,
  Space,
  Typography,
  message,
} from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type {
  ClawTapProbeResponse,
  ClawTapSettings,
  GlobalSettingsResponse,
} from "../../types/globalSettings";
import LlmModelsPage from "./LlmModelsPage";

function formatMs(ms?: number): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

export default function GlobalInferencePage() {
  const { gatewayBase } = useApp();
  const [loading, setLoading] = useState(false);
  const [savingTap, setSavingTap] = useState(false);
  const [probing, setProbing] = useState(false);
  const [probeOk, setProbeOk] = useState(false);
  const [probeDetail, setProbeDetail] = useState<ClawTapProbeResponse | null>(null);
  const [clusterId, setClusterId] = useState("");
  const [tapSettings, setTapSettings] = useState<ClawTapSettings | null>(null);
  const [tapForm] = Form.useForm();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      setClusterId(r.clusterId ?? "");
      const tap = r.clawTap ?? {
        host: "",
        proxyPort: 8080,
        updatedAtMs: 0,
        configured: false,
      };
      setTapSettings(tap);
      tapForm.setFieldsValue({
        host: tap.host,
        proxyPort: tap.proxyPort ?? 8080,
      });
      setProbeOk(false);
      setProbeDetail(null);
    } finally {
      setLoading(false);
    }
  }, [gatewayBase, tapForm]);

  useEffect(() => {
    load().catch(() => {});
  }, [load]);

  const runProbe = async () => {
    const v = await tapForm.validateFields(["host", "proxyPort"]);
    setProbing(true);
    try {
      const resp = await proxyHttp<ClawTapProbeResponse>(
        gatewayBase,
        "POST",
        "/v1/gateway/global-settings/claw-tap/probe",
        {
          host: (v.host as string).trim(),
          proxyPort: v.proxyPort ?? 8080,
        }
      );
      setProbeDetail(resp);
      setProbeOk(resp.ok);
      if (resp.ok) {
        message.success(resp.message);
      } else {
        message.error(resp.message);
      }
    } finally {
      setProbing(false);
    }
  };

  const saveTap = async () => {
    if (!probeOk) {
      message.warning("请先探测并通过校验");
      return;
    }
    const v = await tapForm.validateFields();
    setSavingTap(true);
    try {
      const saved = await proxyHttp<ClawTapSettings>(
        gatewayBase,
        "PUT",
        "/v1/gateway/global-settings/claw-tap",
        {
          host: String(v.host || "").trim(),
          proxyPort: v.proxyPort ?? 8080,
        }
      );
      setTapSettings(saved);
      message.success("clawTap 已保存");
    } finally {
      setSavingTap(false);
    }
  };

  return (
    <div style={{ maxWidth: 960 }}>
      <Typography.Title level={4} style={{ marginTop: 0 }}>
        全局推理
      </Typography.Title>

      <Form layout="vertical" style={{ marginBottom: 16 }}>
        <Form.Item label="集群 ID">
          <Input
            readOnly
            value={clusterId}
            placeholder={loading ? "" : "未设置"}
            style={{ maxWidth: 360, cursor: "default" }}
          />
        </Form.Item>
      </Form>

      <Card title="clawTap 端点（必选）" loading={loading} style={{ marginBottom: 16 }}>
        <Form form={tapForm} layout="vertical" initialValues={{ proxyPort: 8080 }}>
          <Form.Item label="主机" name="host" rules={[{ required: true }]}>
            <Input
              placeholder="192.168.1.10"
              onChange={() => {
                setProbeOk(false);
                setProbeDetail(null);
              }}
            />
          </Form.Item>
          <Form.Item label="代理端口" name="proxyPort" rules={[{ required: true }]}>
            <InputNumber min={1} max={65535} style={{ width: 160 }} />
          </Form.Item>
          {probeDetail ? (
            <Alert
              type={probeOk ? "success" : "error"}
              showIcon
              style={{ marginBottom: 16 }}
              message={probeDetail.message}
            />
          ) : null}
          <Space>
            <Button icon={<ApiOutlined />} loading={probing} onClick={() => runProbe().catch(() => {})}>
              探测
            </Button>
            <Button
              type="primary"
              icon={<SaveOutlined />}
              loading={savingTap}
              onClick={() => saveTap().catch(() => {})}
            >
              保存 clawTap
            </Button>
          </Space>
        </Form>
        {tapSettings?.configured ? (
          <Typography.Paragraph type="secondary" style={{ marginTop: 16 }}>
            上次保存：{formatMs(tapSettings.updatedAtMs)}
          </Typography.Paragraph>
        ) : null}
      </Card>

      <Divider />
      <LlmModelsPage embedded />
    </div>
  );
}
