import { Button, Card, Form, Select, Space, Tag, Typography, message } from "antd";
import { useEffect } from "react";
import { useApp } from "../context/AppContext";
import { putProjectConfigDraft } from "../utils/projectConfig";

const PREFLIGHT_KIND_OPTIONS = [
  {
    value: "sqlbot_mcp_start",
    label: "SQLBot mcp_start（首轮 session 在用户问题后注入 token/chat_id）",
  },
] as const;

export default function PreflightPage() {
  const { gatewayBase, dsId, projectConfig, refreshProjectConfig } = useApp();
  const [form] = Form.useForm<{ kinds: string[] }>();

  useEffect(() => {
    const raw = projectConfig?.solvePreflightJson;
    const kinds = Array.isArray(raw?.kinds)
      ? raw.kinds
      : raw?.kind && raw.kind !== "none"
      ? [raw.kind]
      : [];
    form.setFieldsValue({
      kinds: kinds.filter((k): k is string => typeof k === "string" && k.trim().length > 0),
    });
  }, [projectConfig, form]);

  const orderedKinds = form.getFieldValue("kinds") || [];

  return (
    <Card title="Solve 首轮 Preflight（可顺序执行）" size="small">
      <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
        存于 <Typography.Text code>project_config.solve_preflight_json</Typography.Text>，物化到{" "}
        <Typography.Text code>home/.claw/solve-preflight.json</Typography.Text>。仅该 sessionId
        第一次 solve 执行；续聊不重复。当前按选择顺序执行，后续可继续扩展更多 preflight kind。
      </Typography.Paragraph>
      <Form form={form} layout="vertical">
        <Form.Item
          name="kinds"
          label="执行步骤（按顺序）"
          extra="多选后按选择顺序执行；清空表示不启用 preflight。"
        >
          <Select
            mode="multiple"
            allowClear
            style={{ maxWidth: 760 }}
            options={[...PREFLIGHT_KIND_OPTIONS]}
            placeholder="选择 preflight 步骤"
          />
        </Form.Item>
      </Form>
      <Space style={{ marginBottom: 12 }}>
        <Button
          type="primary"
          onClick={async () => {
            if (!projectConfig) return;
            const v = await form.validateFields();
            const kinds = (v.kinds || []).map((k) => String(k).trim()).filter(Boolean);
            await putProjectConfigDraft(gatewayBase, dsId, projectConfig, {
              solvePreflightJson: { kinds },
            });
            message.success("Preflight 已保存到临时版；设为生效后物化到工作区");
            await refreshProjectConfig();
          }}
        >
          保存 Preflight 配置
        </Button>
      </Space>
      <div>
        {orderedKinds.length === 0 ? (
          <Tag>未启用</Tag>
        ) : (
          orderedKinds.map((k: string, idx: number) => (
            <Tag key={`${k}-${idx}`} color="blue">
              {idx + 1}. {k}
            </Tag>
          ))
        )}
      </div>
    </Card>
  );
}
