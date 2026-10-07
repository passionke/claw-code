import {
  Button,
  Card,
  Form,
  Input,
  Select,
  Space,
  Table,
  Tag,
  Typography,
  message,
} from "antd";
import { DeleteOutlined, PlusOutlined } from "@ant-design/icons";
import { useCallback, useEffect, useState } from "react";
import { useApp } from "../context/AppContext";
import type {
  PreflightLifecycleEvent,
  PreflightPluginRecord,
  PreflightStepJson,
} from "../types/preflight";
import {
  normalizeSolvePreflightSteps,
  resolvedEvent,
  stepsToSolvePreflightJson,
} from "../types/preflight";
import { putProjectConfigDraft } from "../utils/projectConfig";
import { fetchPreflightPlugins, upsertPreflightPlugin } from "../utils/preflightPlugins";

/** Lifecycle event options (`steps[].on`). Author: kejiqing */
const EVENT_OPTIONS: { value: PreflightLifecycleEvent; label: string }[] = [
  { value: "worker.init.start", label: "Worker 首次创建 (worker.init.start)" },
  { value: "worker.init.end", label: "Worker 创建收尾 (worker.init.end)" },
  { value: "worker.reuse.start", label: "Worker 复用/唤醒 (worker.reuse.start)" },
  { value: "worker.reuse.end", label: "Worker 本轮释放前 (worker.reuse.end)" },
  { value: "session.start", label: "Session 首轮 (session.start)" },
  { value: "session.end", label: "Session 关闭 (session.end)" },
  { value: "turn.start", label: "每轮对话开始 (turn.start)" },
  { value: "turn.end", label: "每轮对话结束 (turn.end)" },
];

export default function PreflightPage() {
  const { gatewayBase, projId, projectConfig, refreshProjectConfig } = useApp();
  const [steps, setSteps] = useState<PreflightStepJson[]>([]);
  const [plugins, setPlugins] = useState<PreflightPluginRecord[]>([]);
  const [pluginForm] = Form.useForm<{
    pluginId: string;
    displayName: string;
    command: string;
  }>();

  const loadPlugins = useCallback(async () => {
    try {
      const list = await fetchPreflightPlugins(gatewayBase);
      setPlugins(list);
    } catch (e) {
      message.warning(`加载插件库失败: ${e instanceof Error ? e.message : String(e)}`);
    }
  }, [gatewayBase]);

  useEffect(() => {
    void loadPlugins();
  }, [loadPlugins]);

  useEffect(() => {
    setSteps(normalizeSolvePreflightSteps(projectConfig?.solvePreflightJson));
  }, [projectConfig]);

  const pluginOptions = plugins.map((p) => ({
    value: p.pluginId,
    label: `${p.displayName} (${p.pluginId})`,
  }));

  const addStep = () => {
    const defaultPlugin = plugins[0]?.pluginId ?? "sqlbot_mcp_start";
    setSteps((prev) => [
      ...prev,
      {
        pluginId: defaultPlugin,
        on: "session.start",
        scope: "session_first_turn",
        impl: plugins[0]?.defaultImpl,
      },
    ]);
  };

  const updateStep = (index: number, patch: Partial<PreflightStepJson>) => {
    setSteps((prev) =>
      prev.map((s, i) => (i === index ? { ...s, ...patch } : s)),
    );
  };

  const removeStep = (index: number) => {
    setSteps((prev) => prev.filter((_, i) => i !== index));
  };

  return (
    <Space direction="vertical" size="middle" style={{ width: "100%" }}>
      <Card title="Preflight 插件库（全局）" size="small">
        <Typography.Paragraph type="secondary">
          注册外部子进程插件；内置 <Typography.Text code>turn_language</Typography.Text>、
          <Typography.Text code>sqlbot_mcp_start</Typography.Text> 由迁移种子提供。
        </Typography.Paragraph>
        <Table
          size="small"
          pagination={false}
          rowKey="pluginId"
          dataSource={plugins}
          columns={[
            { title: "pluginId", dataIndex: "pluginId" },
            { title: "名称", dataIndex: "displayName" },
            { title: "SPI", dataIndex: "spiVersion", width: 64 },
            {
              title: "默认实现",
              render: (_, row) =>
                row.defaultImpl?.type === "subprocess"
                  ? (row.defaultImpl.command ?? []).join(" ")
                  : row.defaultImpl?.handler ?? "builtin",
            },
          ]}
        />
        <Form form={pluginForm} layout="inline" style={{ marginTop: 12 }}>
          <Form.Item name="pluginId" rules={[{ required: true }]}>
            <Input placeholder="plugin_id" style={{ width: 160 }} />
          </Form.Item>
          <Form.Item name="displayName" rules={[{ required: true }]}>
            <Input placeholder="展示名称" style={{ width: 200 }} />
          </Form.Item>
          <Form.Item name="command" rules={[{ required: true }]}>
            <Input placeholder="python3 /path/to/plugin.py" style={{ width: 320 }} />
          </Form.Item>
          <Button
            type="default"
            onClick={async () => {
              const v = await pluginForm.validateFields();
              const parts = v.command.trim().split(/\s+/).filter(Boolean);
              await upsertPreflightPlugin(gatewayBase, v.pluginId.trim(), {
                displayName: v.displayName.trim(),
                defaultImpl: { type: "subprocess", command: parts },
              });
              message.success("插件已注册");
              pluginForm.resetFields();
              await loadPlugins();
            }}
          >
            注册子进程插件
          </Button>
        </Form>
      </Card>

      <Card title="项目 Preflight 管道" size="small">
        <Typography.Paragraph type="secondary">
          存于 <Typography.Text code>project_config.solve_preflight_json</Typography.Text>，物化到{" "}
          <Typography.Text code>home/.claw/solve-preflight.json</Typography.Text>。按{" "}
          <Typography.Text code>on</Typography.Text> 生命周期事件过滤执行（见{" "}
          <Typography.Text code>docs/preflight-lifecycle-events.md</Typography.Text>
          ）。<Typography.Text code>worker.init.*</Typography.Text> 需重置 worker 后生效。
        </Typography.Paragraph>
        {steps.length === 0 ? (
          <Tag>未配置步骤（保存为 kind:none；运行时仍默认每轮 turn_language）</Tag>
        ) : (
          steps.map((step, idx) => (
            <Space key={`${step.pluginId}-${idx}`} style={{ display: "flex", marginBottom: 8 }}>
              <Tag color="blue">{idx + 1}</Tag>
              <Select
                style={{ width: 280 }}
                value={step.pluginId}
                options={pluginOptions}
                onChange={(pluginId) => {
                  const plugin = plugins.find((p) => p.pluginId === pluginId);
                  updateStep(idx, {
                    pluginId,
                    impl: plugin?.defaultImpl,
                  });
                }}
              />
              <Select
                style={{ width: 320 }}
                value={resolvedEvent(step)}
                options={EVENT_OPTIONS}
                onChange={(on: PreflightLifecycleEvent) =>
                  updateStep(idx, {
                    on,
                    scope:
                      on === "turn.start"
                        ? "every_turn"
                        : on === "session.start"
                          ? "session_first_turn"
                          : undefined,
                  })
                }
              />
              <Button
                danger
                icon={<DeleteOutlined />}
                onClick={() => removeStep(idx)}
              />
            </Space>
          ))
        )}
        <Space style={{ marginTop: 12 }}>
          <Button icon={<PlusOutlined />} onClick={addStep}>
            添加步骤
          </Button>
          <Button
            type="primary"
            onClick={async () => {
              if (!projectConfig) return;
              await putProjectConfigDraft(gatewayBase, projId, projectConfig, {
                solvePreflightJson: stepsToSolvePreflightJson(steps),
              });
              message.success("Preflight 管道已保存到临时版；设为生效后物化到工作区");
              await refreshProjectConfig();
            }}
          >
            保存项目管道
          </Button>
        </Space>
      </Card>
    </Space>
  );
}
