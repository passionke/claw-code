import { Alert, Button, Card, Form, Radio, Space, Typography, message } from "antd";
import { useEffect } from "react";
import { useApp } from "../context/AppContext";
import { putProjectConfigDraft } from "../utils/projectConfig";
import type { WorkerIsolationJson } from "../types/project";

type Mode = WorkerIsolationJson["mode"];

export default function WorkerIsolationPage() {
  const { gatewayBase, projId, projectConfig, refreshProjectConfig } = useApp();
  const [form] = Form.useForm<{ mode: Mode }>();

  useEffect(() => {
    const raw = projectConfig?.workerIsolationJson?.mode;
    form.setFieldsValue({
      mode: raw === "relaxed" ? "relaxed" : raw === "sandbox" ? "sandbox" : "strict",
    });
  }, [projectConfig, form]);

  return (
    <Card title="Worker 执行环境" size="small">
      <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
        存于 <Typography.Text code>project_config.worker_isolation_json</Typography.Text>
        。按项目决定 worker 后端：本地 podman pool（strict/relaxed）或 FC 云沙箱（sandbox）。
        OVS 交互与 solve_async 共用同一配置。
      </Typography.Paragraph>
      <Alert
        type="info"
        showIcon
        style={{ marginBottom: 16 }}
        message="Sandbox（FC 云沙箱）"
        description={
          <>
            需 gateway 配置 <Typography.Text code>CLAW_FC_*</Typography.Text> 与{" "}
            <Typography.Text code>CLAW_FC_NAS_VOLUME_NAME</Typography.Text>。
            Worker 在阿里云 FC sandbox 内运行，NAS 挂载 <Typography.Text code>/claw_ds</Typography.Text>、
            <Typography.Text code>/claw_host_root</Typography.Text>。
          </>
        }
      />
      <Alert
        type="warning"
        showIcon
        style={{ marginBottom: 16 }}
        message="Relaxed 模式（podman pool）"
        description={
          <>
            容器内 root、可写 rootfs（跳过 CLAW_SECURITY_BOOST）、不 chmod 锁配置副本。
            生产可设 <Typography.Text code>CLAW_ALLOW_RELAXED_WORKER=false</Typography.Text>{" "}
            全局禁用。
          </>
        }
      />
      <Form form={form} layout="vertical">
        <Form.Item name="mode" label="模式">
          <Radio.Group>
            <Radio value="strict">Strict（podman pool：security_boost + uid 1000）</Radio>
            <Radio value="relaxed">Relaxed（podman pool：root + 可写容器）</Radio>
            <Radio value="sandbox">Sandbox（FC 云沙箱）</Radio>
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
              workerIsolationJson: { mode: v.mode },
            });
            message.success("Worker 执行环境已保存；下次 acquire / terminal start 时生效");
            await refreshProjectConfig();
          }}
        >
          保存 Worker 配置
        </Button>
      </Space>
    </Card>
  );
}
