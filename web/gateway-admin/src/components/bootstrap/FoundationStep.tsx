import { Alert, Button, Form, Input, Space, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type {
  BootstrapApplyDeployEnvResponse,
  BootstrapEnvSnapshot,
  ClusterBootstrapSnapshot,
} from "../../types/globalSettings";

type Props = {
  snap: ClusterBootstrapSnapshot;
  onRefresh: () => Promise<ClusterBootstrapSnapshot | null>;
  onNext: () => void;
};

/** Step 1: 下一步 = 写 .env + 运行时替换 e2b。Author: kejiqing */
export default function FoundationStep({ snap: _snap, onRefresh, onNext }: Props) {
  const { gatewayBase } = useApp();
  const [form] = Form.useForm();
  const [envSnap, setEnvSnap] = useState<BootstrapEnvSnapshot | null>(null);
  const [saving, setSaving] = useState(false);
  const [needsRestart, setNeedsRestart] = useState(false);
  const [lastValidation, setLastValidation] = useState<BootstrapApplyDeployEnvResponse["validation"] | null>(
    null
  );

  const loadEnv = useCallback(async () => {
    if (!gatewayBase) return;
    try {
      const data = await proxyHttp<BootstrapEnvSnapshot>(
        gatewayBase,
        "GET",
        "/v1/gateway/bootstrap/env-snapshot"
      );
      setEnvSnap(data);
      form.setFieldsValue({
        CLAW_CLUSTER_ID: data.values.CLAW_CLUSTER_ID ?? data.clusterId,
        CLAW_E2B_API_URL: data.values.CLAW_E2B_API_URL ?? data.e2bPlatform.e2bApiUrl,
        CLAW_E2B_API_KEY: "",
        CLAW_E2B_DOMAIN: data.values.CLAW_E2B_DOMAIN ?? data.e2bPlatform.e2bDomain,
        CLAW_DEPLOY_PROFILE: data.values.CLAW_DEPLOY_PROFILE ?? data.deployProfile ?? "local",
      });
    } catch (e) {
      message.error(e instanceof Error ? e.message : "加载环境快照失败");
    }
  }, [form, gatewayBase]);

  useEffect(() => {
    void loadEnv();
  }, [loadEnv]);

  const next = async () => {
    if (!gatewayBase) return;
    const values = await form.validateFields();
    const payload: Record<string, string> = {};
    for (const [k, v] of Object.entries(values)) {
      if (typeof v === "string" && v.trim()) payload[k] = v.trim();
    }
    setSaving(true);
    try {
      const resp = await proxyHttp<BootstrapApplyDeployEnvResponse>(
        gatewayBase,
        "POST",
        "/v1/gateway/bootstrap/apply-deploy-env",
        { values: payload }
      );
      setLastValidation(resp.validation);
      setNeedsRestart(resp.restartRequired);
      await onRefresh();
      await loadEnv();
      if (resp.restartRequired) {
        message.warning("已写入 .env 且 e2b 已热替换；cluster/PG 变更需重启 Gateway 后再继续");
        return;
      }
      if (resp.templatesInvalidated) {
        message.warning("e2b 端点已变更：旧模板 buildId 已清空，请重新「发布模板」");
      } else {
        message.success("已写入 .env 并生效");
      }
      onNext();
    } catch (e) {
      message.error(e instanceof Error ? e.message : "写入失败");
    } finally {
      setSaving(false);
    }
  };

  return (
    <Space direction="vertical" size="middle" style={{ width: "100%" }}>
      <Typography.Paragraph type="secondary">
        填写集群 ID 与 e2b 连接；点「下一步」自动写入 deploy .env，并立即替换进程内 e2b 连接。
      </Typography.Paragraph>

      {envSnap?.pgHostPort ? (
        <Alert
          type="info"
          showIcon
          message={`PostgreSQL：${envSnap.pgHostPort}（共用库，系统托管）`}
          description="运维已在 .env 预置 PG 连接；你只需设 CLAW_CLUSTER_ID，下一步时自动同步 RLS。"
        />
      ) : null}

      {!envSnap?.deployEnvWritable ? (
        <Alert
          type="warning"
          showIcon
          message="deploy .env 不可写"
          description="请确认 compose 已挂载 /run/claw/deploy.env:rw 后重启 Gateway 容器。"
        />
      ) : null}

      <Form form={form} layout="vertical" disabled={!envSnap?.deployEnvWritable}>
        <Form.Item
          name="CLAW_CLUSTER_ID"
          label="CLAW_CLUSTER_ID"
          rules={[{ required: true, message: "必填" }]}
          extra="新集群唯一名称，例如 workbox-20260828"
        >
          <Input placeholder="workbox-20260828" />
        </Form.Item>
        <Form.Item name="CLAW_E2B_API_URL" label="CLAW_E2B_API_URL" rules={[{ required: true }]}>
          <Input />
        </Form.Item>
        <Form.Item name="CLAW_E2B_API_KEY" label="CLAW_E2B_API_KEY" extra="留空则不修改已有 key">
          <Input.Password placeholder={envSnap?.e2bPlatform.apiKeySet ? "已配置（输入新值覆盖）" : ""} />
        </Form.Item>
        <Form.Item name="CLAW_E2B_DOMAIN" label="CLAW_E2B_DOMAIN">
          <Input />
        </Form.Item>
        <Form.Item name="CLAW_DEPLOY_PROFILE" label="CLAW_DEPLOY_PROFILE">
          <Input placeholder="local / production" />
        </Form.Item>
      </Form>

      {lastValidation ? (
        <Alert
          type={lastValidation.pgOk && lastValidation.e2bOk ? "success" : "warning"}
          showIcon
          message={`PG: ${lastValidation.pgOk ? "通" : "失败"} · e2b: ${lastValidation.e2bOk ? "通" : "失败"}`}
          description={lastValidation.message}
        />
      ) : null}

      {needsRestart ? (
        <Alert
          type="warning"
          showIcon
          message="需重启 Gateway（cluster / PG 绑定）"
          description="e2b 已热替换；CLAW_CLUSTER_ID 变更同步了 PG RLS URL 时，请在 deploy host 执行：./deploy/stack/gateway.sh up，再点下一步。"
        />
      ) : null}

      <Space wrap>
        <Button
          type="primary"
          loading={saving}
          disabled={!envSnap?.deployEnvWritable}
          onClick={() => void next()}
        >
          下一步
        </Button>
        <Button onClick={() => void loadEnv()}>重新加载</Button>
        <Button onClick={() => void onRefresh()}>重新检测</Button>
      </Space>

      <Typography.Text type="secondary">
        当前 PG（进程内）: <code>{envSnap?.gatewayDatabaseUrl ?? "—"}</code>
        {envSnap?.pgRlsManaged ? " · RLS 由下一步写 clusterId 时自动维护" : null}
      </Typography.Text>
    </Space>
  );
}
