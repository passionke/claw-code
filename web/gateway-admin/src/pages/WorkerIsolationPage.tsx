import { Alert, Button, Card, Form, Radio, Space, Typography, message } from "antd";
import { useEffect } from "react";
import { useApp } from "../context/AppContext";
import { putProjectConfigDraft } from "../utils/projectConfig";
import type { WorkerIsolationJson } from "../types/project";

type Mode = WorkerIsolationJson["mode"];

export default function WorkerIsolationPage() {
  const { gatewayBase, dsId, projectConfig, refreshProjectConfig } = useApp();
  const [form] = Form.useForm<{ mode: Mode }>();

  useEffect(() => {
    const raw = projectConfig?.workerIsolationJson?.mode;
    form.setFieldsValue({
      mode: raw === "relaxed" ? "relaxed" : "strict",
    });
  }, [projectConfig, form]);

  return (
    <Card title="Worker 隔离模式" size="small">
      <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
        存于 <Typography.Text code>project_config.worker_isolation_json</Typography.Text>
        。由 pool-daemon 在容器 <Typography.Text code>run</Typography.Text> /{" "}
        <Typography.Text code>exec</Typography.Text> 时施加；worker 进程不可感知 mode。
        保存后下次 acquire 生效（可能重建 worker 容器）。
      </Typography.Paragraph>
      <Alert
        type="warning"
        showIcon
        style={{ marginBottom: 16 }}
        message="Relaxed 模式"
        description={
          <>
            容器内 root、可写 rootfs（跳过 CLAW_SECURITY_BOOST）、不 chmod 锁配置副本。
            网络 egress 与 strict 相同（均开放）。边界：/claw_ds 只读 bind、无 docker.sock、会话
            tmpfs。生产可设 <Typography.Text code>CLAW_ALLOW_RELAXED_WORKER=false</Typography.Text>{" "}
            全局禁用。
          </>
        }
      />
      <Form form={form} layout="vertical">
        <Form.Item name="mode" label="模式">
          <Radio.Group>
            <Radio value="strict">Strict（默认：security_boost + uid 1000 + guest_lock）</Radio>
            <Radio value="relaxed">Relaxed（root + 可写容器 + 无 guest_lock）</Radio>
          </Radio.Group>
        </Form.Item>
      </Form>
      <Space>
        <Button
          type="primary"
          onClick={async () => {
            if (!projectConfig) return;
            const v = await form.validateFields();
            await putProjectConfigDraft(gatewayBase, dsId, projectConfig, {
              workerIsolationJson: { mode: v.mode },
            });
            message.success("Worker 隔离已保存；下次 solve acquire 时生效");
            await refreshProjectConfig();
          }}
        >
          保存 Worker 隔离
        </Button>
      </Space>
    </Card>
  );
}
