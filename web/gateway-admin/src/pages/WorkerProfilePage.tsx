import { Alert, Button, Card, Form, Radio, Space, Typography, message } from "antd";
import { useEffect } from "react";
import { useApp } from "../context/AppContext";
import { putProjectConfigDraft } from "../utils/projectConfig";
import type { WorkerProfileJson } from "../types/project";

type Mode = WorkerProfileJson["mode"];

export default function WorkerProfilePage() {
  const { gatewayBase, projId, projectConfig, refreshProjectConfig } = useApp();
  const [form] = Form.useForm<{ mode: Mode }>();

  useEffect(() => {
    const raw = projectConfig?.workerProfileJson?.mode;
    form.setFieldsValue({
      mode: raw === "relaxed" ? "relaxed" : "strict",
    });
  }, [projectConfig, form]);

  return (
    <Card title="Worker 执行环境" size="small">
      <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
        存于 <Typography.Text code>project_config.worker_profile_json</Typography.Text>
        。Worker 始终在 <Typography.Text strong>e2b</Typography.Text> 沙箱运行；此处仅选择
        worker profile。OVS 与 solve_async 共用同一配置。
      </Typography.Paragraph>
      <Alert
        type="info"
        showIcon
        style={{ marginBottom: 16 }}
        message="e2b 基座（系统默认）"
        description={
          <>
            需 gateway 配置 <Typography.Text code>CLAW_E2B_*</Typography.Text> 与 NAS API。
            Worker 在 e2b sandbox 内运行，NAS 挂载 <Typography.Text code>/claw_ds</Typography.Text>、
            <Typography.Text code>/claw_host_root</Typography.Text>。
          </>
        }
      />
      <Alert
        type="warning"
        showIcon
        style={{ marginBottom: 16 }}
        message="Relaxed profile（OVS 模式）"
        description={
          <>
            guest root、可写 rootfs（跳过 CLAW_SECURITY_BOOST）、不 chmod 锁配置副本。
            生产可设 <Typography.Text code>CLAW_ALLOW_RELAXED_WORKER=false</Typography.Text>{" "}
            全局禁用。
          </>
        }
      />
      <Form form={form} layout="vertical">
        <Form.Item name="mode" label="Worker profile">
          <Radio.Group>
            <Radio value="strict">Strict（对话模式：security_boost + uid 1000）</Radio>
            <Radio value="relaxed">Relaxed（OVS 模式：root + 可写容器）</Radio>
          </Radio.Group>
        </Form.Item>
      </Form>
      <Space>
        <Button
          type="primary"
          onClick={async () => {
            if (!projectConfig) return;
            const v = await form.validateFields();
            await putProjectConfigDraft(gatewayBase, projId, projectConfig, {
              workerProfileJson: { mode: v.mode },
            });
            message.success("Worker profile 已保存；下次 acquire / terminal start 时生效");
            await refreshProjectConfig();
          }}
        >
          保存 Worker profile
        </Button>
      </Space>
    </Card>
  );
}
