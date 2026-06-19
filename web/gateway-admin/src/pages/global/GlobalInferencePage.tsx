import { ApiOutlined, CloudServerOutlined, SaveOutlined } from "@ant-design/icons";
import {
  Alert,
  Button,
  Card,
  Divider,
  Form,
  Input,
  InputNumber,
  Space,
  Tabs,
  Typography,
  message,
} from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type {
  ClawTapMode,
  ClawTapProbeResponse,
  ClawTapSettings,
  GlobalSettingsResponse,
  PutClawTapSettingsResponse,
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
  const [tapMode, setTapMode] = useState<ClawTapMode>("local");
  const [tapSettings, setTapSettings] = useState<ClawTapSettings | null>(null);
  const [localForm] = Form.useForm();
  const [remoteForm] = Form.useForm();

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
        mode: "local" as ClawTapMode,
        host: "",
        proxyPort: 8080,
        livePort: 3000,
        updatedAtMs: 0,
        configured: false,
      };
      const mode = tap.mode ?? "local";
      setTapMode(mode);
      setTapSettings(tap);
      localForm.setFieldsValue({ livePort: tap.livePort ?? 3000 });
      remoteForm.setFieldsValue({
        host: tap.host,
        proxyPort: tap.proxyPort ?? 8080,
      });
      setProbeOk(false);
      setProbeDetail(null);
    } finally {
      setLoading(false);
    }
  }, [gatewayBase, localForm, remoteForm]);

  useEffect(() => {
    load().catch(() => {});
  }, [load]);

  const runProbe = async () => {
    const v = await remoteForm.validateFields(["host", "proxyPort"]);
    setProbing(true);
    try {
      const resp = await proxyHttp<ClawTapProbeResponse>(
        gatewayBase,
        "POST",
        "/v1/gateway/global-settings/claw-tap/probe",
        {
          mode: "remote",
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

  const saveLocalTap = async () => {
    const v = await localForm.validateFields();
    setSavingTap(true);
    try {
      const saved = await proxyHttp<PutClawTapSettingsResponse>(
        gatewayBase,
        "PUT",
        "/v1/gateway/global-settings/claw-tap",
        {
          mode: "local",
          livePort: v.livePort ?? 3000,
        }
      );
      setTapSettings(saved);
      setTapMode("local");
      if (saved.message) {
        message.warning(saved.message);
      } else if (saved.tapRestart?.restarted) {
        message.success("本地 clawTap 已保存并重启");
      } else {
        message.success("本地 clawTap 已保存");
      }
      await load();
    } finally {
      setSavingTap(false);
    }
  };

  const saveRemoteTap = async () => {
    if (!probeOk) {
      message.warning("请先探测并通过校验");
      return;
    }
    const v = await remoteForm.validateFields();
    setSavingTap(true);
    try {
      const saved = await proxyHttp<PutClawTapSettingsResponse>(
        gatewayBase,
        "PUT",
        "/v1/gateway/global-settings/claw-tap",
        {
          mode: "remote",
          host: String(v.host || "").trim(),
          proxyPort: v.proxyPort ?? 8080,
        }
      );
      setTapSettings(saved);
      setTapMode("remote");
      message.success("远端 clawTap 已保存");
      await load();
    } finally {
      setSavingTap(false);
    }
  };

  const localTab = (
    <div>
      <Typography.Paragraph type="secondary">
        Gateway 在本机管理 claude-tap 侧车；切换大模型时会自动重启 tap。代理端口固定 8080（容器内
        <Typography.Text code>claw-claude-tap</Typography.Text>
        ），无需手动填写。
      </Typography.Paragraph>
      <Form form={localForm} layout="vertical" initialValues={{ livePort: 3000 }}>
        <Form.Item
          label="Live 端口（浏览器访问 trace viewer）"
          name="livePort"
          rules={[{ required: true }]}
        >
          <InputNumber min={1} max={65535} style={{ width: 160 }} />
        </Form.Item>
        <Button
          type="primary"
          icon={<SaveOutlined />}
          loading={savingTap}
          onClick={() => saveLocalTap().catch((e) => message.error(String(e)))}
        >
          保存本地部署
        </Button>
      </Form>
      {tapSettings?.configured && tapMode === "local" ? (
        <Typography.Paragraph type="secondary" style={{ marginTop: 16 }}>
          上次保存：{formatMs(tapSettings.updatedAtMs)}
          {tapSettings.liveBaseUrl ? (
            <>
              <br />
              Live：{tapSettings.liveBaseUrl}
            </>
          ) : null}
          {tapSettings.proxyBaseUrl ? (
            <>
              <br />
              代理（内部）：{tapSettings.proxyBaseUrl}
            </>
          ) : null}
        </Typography.Paragraph>
      ) : null}

      <Divider />
      <LlmModelsPage embedded />
    </div>
  );

  const remoteTab = (
    <div>
      <Typography.Paragraph type="secondary">
        连接集群共享或远程 claude-tap；LLM 上游由远端 tap 自行管理，此处无需配置大模型。
      </Typography.Paragraph>
      <Form form={remoteForm} layout="vertical" initialValues={{ proxyPort: 8080 }}>
        <Form.Item label="主机 / IP" name="host" rules={[{ required: true }]}>
          <Input
            placeholder="10.22.28.94"
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
            onClick={() => saveRemoteTap().catch((e) => message.error(String(e)))}
          >
            保存远端 clawTap
          </Button>
        </Space>
      </Form>
      {tapSettings?.configured && tapMode === "remote" ? (
        <Typography.Paragraph type="secondary" style={{ marginTop: 16 }}>
          上次保存：{formatMs(tapSettings.updatedAtMs)}
          {tapSettings.host ? (
            <>
              <br />
              端点：{tapSettings.host}:{tapSettings.proxyPort}
            </>
          ) : null}
        </Typography.Paragraph>
      ) : null}
    </div>
  );

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

      <Card title="clawTap 端点（必选）" loading={loading}>
        <Tabs
          activeKey={tapMode}
          onChange={(k) => setTapMode(k as ClawTapMode)}
          items={[
            {
              key: "local",
              label: (
                <span>
                  <CloudServerOutlined /> 本地部署
                </span>
              ),
              children: localTab,
            },
            {
              key: "remote",
              label: (
                <span>
                  <ApiOutlined /> 远端 tap
                </span>
              ),
              children: remoteTab,
            },
          ]}
        />
      </Card>
    </div>
  );
}
