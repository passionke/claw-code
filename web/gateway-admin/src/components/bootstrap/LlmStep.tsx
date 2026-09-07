import { Alert, Button, Form, Input, Space, Typography, message } from "antd";
import { useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type { ClusterBootstrapSnapshot } from "../../types/globalSettings";

type Props = {
  snap: ClusterBootstrapSnapshot;
  onRefresh: () => Promise<ClusterBootstrapSnapshot | null>;
  onNext: () => void;
};

/** Step 2: inline LLM form — test + apply to PG. Author: kejiqing */
export default function LlmStep({ snap, onRefresh, onNext }: Props) {
  const { gatewayBase } = useApp();
  const [form] = Form.useForm();
  const [applying, setApplying] = useState(false);

  const llmOk = snap.phases.find((p) => p.phase === "llm_config")?.complete;

  const apply = async () => {
    if (!gatewayBase) return;
    const v = await form.validateFields();
    setApplying(true);
    try {
      await proxyHttp(gatewayBase, "PUT", "/v1/gateway/global-settings/active-llm-config", {
        name: v.name,
        baseModelUrl: v.baseModelUrl,
        modelName: v.modelName,
        apiKey: v.apiKey,
        note: "cluster bootstrap wizard",
      });
      message.success("LLM 已写入 PG 并 Apply");
      const updated = await onRefresh();
      if (updated?.phases.find((p) => p.phase === "llm_config")?.complete) {
        onNext();
      }
    } catch (e) {
      message.error(e instanceof Error ? e.message : "Apply 失败");
    } finally {
      setApplying(false);
    }
  };

  const applyFromEnv = async () => {
    if (!gatewayBase) return;
    setApplying(true);
    try {
      const resp = await proxyHttp<{ applied: boolean; message?: string }>(
        gatewayBase,
        "POST",
        "/v1/gateway/bootstrap/apply-llm-from-env"
      );
      if (resp.applied) message.success("已从 env 应用 LLM");
      else message.info(resp.message ?? "未应用");
      await onRefresh();
    } catch (e) {
      message.error(e instanceof Error ? e.message : "从 env 应用失败");
    } finally {
      setApplying(false);
    }
  };

  return (
    <Space direction="vertical" size="middle" style={{ width: "100%" }}>
      <Typography.Paragraph type="secondary">
        配置全局推理模型并写入 PostgreSQL。「下一步」仅在 active LLM 已生效后出现；点它即表示本步成功。
      </Typography.Paragraph>

      {llmOk ? (
        <Alert type="success" showIcon message="active LLM 已就绪，可进入下一步" />
      ) : (
        <Alert type="info" showIcon message="请先「保存并 Apply」；成功后才会出现「下一步」" />
      )}

      {snap.envLlmAvailable ? (
        <Alert
          type="info"
          showIcon
          message="检测到 CLAW_BOOTSTRAP_LLM_*"
          action={
            <Button size="small" loading={applying} onClick={() => void applyFromEnv()}>
              从 env 应用
            </Button>
          }
        />
      ) : null}

      <Form
        form={form}
        layout="vertical"
        initialValues={{
          name: "bootstrap-llm",
          baseModelUrl: "https://api.deepseek.com/v1",
          modelName: "deepseek-chat",
        }}
      >
        <Form.Item name="name" label="显示名称" rules={[{ required: true }]}>
          <Input />
        </Form.Item>
        <Form.Item name="baseModelUrl" label="Base URL" rules={[{ required: true }]}>
          <Input />
        </Form.Item>
        <Form.Item name="modelName" label="Model" rules={[{ required: true }]}>
          <Input />
        </Form.Item>
        <Form.Item name="apiKey" label="API Key" rules={[{ required: true }]}>
          <Input.Password />
        </Form.Item>
      </Form>

      <Space wrap>
        <Button type="primary" loading={applying} onClick={() => void apply()}>
          保存并 Apply
        </Button>
        {llmOk ? (
          <Button type="primary" onClick={onNext}>
            下一步
          </Button>
        ) : null}
      </Space>
    </Space>
  );
}
