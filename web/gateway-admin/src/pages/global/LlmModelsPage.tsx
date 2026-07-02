import { CloudOutlined, PlusOutlined, ThunderboltOutlined } from "@ant-design/icons";
import {
  Alert,
  Button,
  Card,
  Collapse,
  Form,
  Input,
  InputNumber,
  Modal,
  Popconfirm,
  Select,
  Space,
  Spin,
  Table,
  Tag,
  Typography,
  message,
} from "antd";
import type { ColumnsType } from "antd/es/table";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type { GlobalSettingsResponse, LlmModelRow } from "../../types/globalSettings";
import type { LlmTestResponse, ThinkingMode } from "../../types/llmTest";
import { testLlmModel, thinkingModeToApi } from "../../utils/llmTest";
import {
  findLlmPresetByEndpoint,
  groupLlmPresetsByProvider,
  LLM_PROVIDER_CUSTOM_ID,
  LLM_PROVIDER_PRESETS,
  type LlmProviderPreset,
} from "../../utils/llmProviders";

const DEFAULT_TEST_PROMPT = "Reply with exactly: pong";

const PROVIDER_GROUPS = groupLlmPresetsByProvider(LLM_PROVIDER_PRESETS);
const DEFAULT_PRESET_ID = LLM_PROVIDER_PRESETS.find((p) => p.presetId === "deepseek-v4-flash")?.presetId
  ?? LLM_PROVIDER_PRESETS[0]?.presetId
  ?? LLM_PROVIDER_CUSTOM_ID;

function applyPresetToForm(
  form: ReturnType<typeof Form.useForm>[0],
  preset: LlmProviderPreset
) {
  form.setFieldsValue({
    name: preset.displayName,
    baseModelUrl: preset.baseModelUrl,
    modelName: preset.modelId,
  });
}

function presetIdForRow(row: LlmModelRow): string {
  return findLlmPresetByEndpoint(row.baseModelUrl, row.modelName)?.presetId ?? LLM_PROVIDER_CUSTOM_ID;
}

function formatMs(ms?: number): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

export default function LlmModelsPage({ embedded = false }: { embedded?: boolean }) {
  const { gatewayBase } = useApp();
  const [models, setModels] = useState<LlmModelRow[]>([]);
  const [activeLlmModelId, setActiveLlmModelId] = useState<string | undefined>();
  const [activeLlmModelRev, setActiveLlmModelRev] = useState<string | undefined>();
  const [activeLlmConfig, setActiveLlmConfig] = useState<
    GlobalSettingsResponse["activeLlmConfig"]
  >();
  const [activeLlmAppliedAtMs, setActiveLlmAppliedAtMs] = useState<number | undefined>();
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [editing, setEditing] = useState<LlmModelRow | null>(null);
  const [form] = Form.useForm();
  const presetId = Form.useWatch("presetId", form);
  const isCustomPreset = !presetId || presetId === LLM_PROVIDER_CUSTOM_ID;
  const [testModalOpen, setTestModalOpen] = useState(false);
  const [testingRow, setTestingRow] = useState<LlmModelRow | null>(null);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<LlmTestResponse | null>(null);
  const [testForm] = Form.useForm<{
    prompt: string;
    thinkingMode: ThinkingMode;
    temperature?: number;
    topP?: number;
    maxTokens?: number;
    frequencyPenalty?: number;
    presencePenalty?: number;
    reasoningEffort?: string;
  }>();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      setModels(r.llmModels || []);
      setActiveLlmModelId(r.activeLlmModelId);
      setActiveLlmModelRev(r.activeLlmModelRev);
      setActiveLlmConfig(r.activeLlmConfig);
      setActiveLlmAppliedAtMs(r.activeLlmAppliedAtMs);
    } finally {
      setLoading(false);
    }
  }, [gatewayBase]);

  useEffect(() => {
    load().catch(() => {
      setModels([]);
      setActiveLlmModelId(undefined);
      setActiveLlmModelRev(undefined);
      setActiveLlmConfig(undefined);
    });
  }, [load]);

  const openCreate = () => {
    setEditing(null);
    form.resetFields();
    const preset = LLM_PROVIDER_PRESETS.find((p) => p.presetId === DEFAULT_PRESET_ID);
    form.setFieldsValue({ presetId: DEFAULT_PRESET_ID });
    if (preset) applyPresetToForm(form, preset);
    setModalOpen(true);
  };

  const openEdit = (row: LlmModelRow) => {
    setEditing(row);
    const pid = presetIdForRow(row);
    form.setFieldsValue({
      presetId: pid,
      name: row.name,
      baseModelUrl: row.baseModelUrl,
      modelName: row.modelName,
      apiKey: "",
    });
    setModalOpen(true);
  };

  const onPresetChange = (id: string) => {
    if (id === LLM_PROVIDER_CUSTOM_ID) return;
    const preset = LLM_PROVIDER_PRESETS.find((p) => p.presetId === id);
    if (preset) applyPresetToForm(form, preset);
  };

  const saveModel = async () => {
    const v = await form.validateFields();
    const name = (v.name || "").trim();
    const baseModelUrl = (v.baseModelUrl || "").trim();
    const modelName = (v.modelName || "").trim();
    const apiKey = (v.apiKey || "").trim();
    if (!name || !baseModelUrl || !modelName) {
      message.error("请填写名称、Base URL 与模型 ID");
      return;
    }
    if (!editing && !apiKey) {
      message.error("新建模型必须填写 API Key");
      return;
    }
    setSaving(true);
    try {
      const body: {
        id?: string;
        name: string;
        baseModelUrl: string;
        modelName: string;
        apiKey?: string;
      } = { name, baseModelUrl, modelName };
      if (editing) body.id = editing.id;
      if (apiKey) body.apiKey = apiKey;
      await proxyHttp<LlmModelRow>(
        gatewayBase,
        "POST",
        "/v1/gateway/global-settings/llm-models",
        body
      );
      message.success(editing ? "模型已更新" : "模型已添加");
      setModalOpen(false);
      await load();
    } catch (e) {
      message.error(String(e));
    } finally {
      setSaving(false);
    }
  };

  const applyModel = async (row: LlmModelRow) => {
    const resp = await proxyHttp<{
      outcome?: { tapRestarted?: boolean; message?: string };
    }>(
      gatewayBase,
      "POST",
      `/v1/gateway/global-settings/llm-models/${encodeURIComponent(row.id)}/apply`
    );
    const restarted = resp.outcome?.tapRestarted;
    const detail = resp.outcome?.message;
    if (restarted) {
      message.success(`已切换为当前模型：${row.name}（tap 已重启）`);
    } else if (detail) {
      message.success(`已切换为当前模型：${row.name}（${detail}）`);
    } else {
      message.success(`已切换为当前模型：${row.name}`);
    }
    await load();
  };

  const openTest = (row: LlmModelRow) => {
    setTestingRow(row);
    setTestResult(null);
    testForm.setFieldsValue({
      prompt: DEFAULT_TEST_PROMPT,
      thinkingMode: "default",
      temperature: undefined,
      topP: undefined,
      maxTokens: 256,
      frequencyPenalty: undefined,
      presencePenalty: undefined,
      reasoningEffort: undefined,
    });
    setTestModalOpen(true);
  };

  const runTest = async () => {
    if (!testingRow) return;
    if (!testingRow.apiKeySet) {
      message.warning("请先配置 API Key 后再测试");
      return;
    }
    const v = await testForm.validateFields();
    setTesting(true);
    setTestResult(null);
    try {
      const thinkingEnabled = thinkingModeToApi(v.thinkingMode);
      const req: Parameters<typeof testLlmModel>[1] = {
        modelId: testingRow.id,
        prompt: (v.prompt || "").trim() || DEFAULT_TEST_PROMPT,
      };
      if (thinkingEnabled !== undefined) req.thinkingEnabled = thinkingEnabled;
      if (typeof v.temperature === "number") req.temperature = v.temperature;
      if (typeof v.topP === "number") req.topP = v.topP;
      if (typeof v.maxTokens === "number") req.maxTokens = v.maxTokens;
      if (typeof v.frequencyPenalty === "number") req.frequencyPenalty = v.frequencyPenalty;
      if (typeof v.presencePenalty === "number") req.presencePenalty = v.presencePenalty;
      const effort = (v.reasoningEffort || "").trim();
      if (effort) req.reasoningEffort = effort;

      const r = await testLlmModel(gatewayBase, req);
      setTestResult(r);
      if (r.ok) {
        message.success(`模型「${testingRow.name}」测试通过（${r.durationMs}ms）`);
      } else {
        message.error(`模型「${testingRow.name}」测试未通过`);
      }
    } catch (e) {
      message.error(String(e));
    } finally {
      setTesting(false);
    }
  };

  const deleteModel = async (row: LlmModelRow) => {
    await proxyHttp(
      gatewayBase,
      "DELETE",
      `/v1/gateway/global-settings/llm-models/${encodeURIComponent(row.id)}`
    );
    message.success("已删除");
    await load();
  };

  const runtimeActiveModelId = activeLlmConfig?.modelId;
  const runtimeNeedsReapply = Boolean(
    activeLlmModelId && runtimeActiveModelId !== activeLlmModelId
  );

  const columns: ColumnsType<LlmModelRow> = [
    { title: "名称", dataIndex: "name", width: 140 },
    {
      title: "状态",
      width: 120,
      render: (_, row) => {
        const markedActive = row.active || row.id === activeLlmModelId;
        const runtimeActive = runtimeActiveModelId === row.id;
        if (runtimeActive) {
          return <Tag color="success">运行中</Tag>;
        }
        if (markedActive) {
          return <Tag color="warning">待同步</Tag>;
        }
        return <Tag>—</Tag>;
      },
    },
    {
      title: "Base URL",
      dataIndex: "baseModelUrl",
      ellipsis: true,
    },
    { title: "模型 ID", dataIndex: "modelName", width: 160, ellipsis: true },
    {
      title: "API Key",
      width: 90,
      render: (_, row) =>
        row.apiKeySet ? <Tag color="green">已配置</Tag> : <Tag>未配置</Tag>,
    },
    {
      title: "操作",
      width: 280,
      render: (_, row) => {
        const markedActive = row.active || row.id === activeLlmModelId;
        const runtimeActive = runtimeActiveModelId === row.id;
        return (
          <Space wrap>
            <Button
              size="small"
              icon={<ThunderboltOutlined />}
              disabled={!row.apiKeySet}
              onClick={() => openTest(row)}
            >
              测试
            </Button>
            <Button size="small" onClick={() => openEdit(row)}>
              编辑
            </Button>
            <Button
              size="small"
              type="primary"
              disabled={runtimeActive}
              onClick={() => applyModel(row).catch((e) => message.error(String(e)))}
            >
              设为当前
            </Button>
            <Popconfirm
              title="删除此模型？"
              description={markedActive ? "当前生效模型删除后将无全局 LLM。" : undefined}
              onConfirm={() => deleteModel(row).catch((e) => message.error(String(e)))}
            >
              <Button size="small" danger>
                删除
              </Button>
            </Popconfirm>
          </Space>
        );
      },
    },
  ];

  return (
    <div>
      {!embedded ? (
        <Typography.Title level={4} style={{ marginTop: 0 }}>
          模型配置
        </Typography.Title>
      ) : (
        <Typography.Title level={5}>大模型列表</Typography.Title>
      )}

      <Card
        title={
          embedded ? undefined : (
          <Space>
            <CloudOutlined />
            <span>大模型列表</span>
            {runtimeActiveModelId ? (
              <Tag color="success">运行中</Tag>
            ) : activeLlmModelId ? (
              <Tag color="warning">待同步</Tag>
            ) : (
              <Tag>未设当前</Tag>
            )}
          </Space>
          )
        }
        size="small"
        extra={
          <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>
            添加模型
          </Button>
        }
        loading={loading}
      >
        {runtimeNeedsReapply ? (
          <Alert
            type="warning"
            showIcon
            style={{ marginBottom: 12 }}
            message="当前模型配置未同步到运行时"
            description={
              <>
                列表中已标记为当前的模型（
                <Typography.Text code>{activeLlmModelId}</Typography.Text>
                {activeLlmModelRev ? (
                  <>
                    ，revision <Typography.Text code>{activeLlmModelRev}</Typography.Text>
                  </>
                ) : null}
                ）无法被 solve 使用。请点击「设为当前」完成同步。
              </>
            }
          />
        ) : null}
        {activeLlmAppliedAtMs ? (
          <Typography.Text type="secondary" style={{ display: "block", marginBottom: 12 }}>
            最近切换：{formatMs(activeLlmAppliedAtMs)}
          </Typography.Text>
        ) : null}
        <Table
          rowKey="id"
          size="small"
          columns={columns}
          dataSource={models}
          pagination={false}
          locale={{ emptyText: "暂无模型，点击「添加模型」" }}
        />
      </Card>

      <Modal
        title={editing ? `编辑模型 · ${editing.name}` : "添加模型"}
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={() => saveModel()}
        confirmLoading={saving}
        destroyOnClose
        width={520}
      >
        <Form form={form} layout="vertical">
          <Form.Item
            name="presetId"
            label="服务商 / 模型"
            rules={[{ required: true, message: "请选择服务商" }]}
            tooltip="预设来自 llm-providers.csv；仅需填写 API Key"
          >
            <Select
              showSearch
              optionFilterProp="label"
              onChange={onPresetChange}
              options={[
                ...PROVIDER_GROUPS.map((g) => ({
                  label: g.providerLabel,
                  options: g.presets.map((p) => ({
                    value: p.presetId,
                    label: p.modelId,
                  })),
                })),
                {
                  label: "其他",
                  options: [{ value: LLM_PROVIDER_CUSTOM_ID, label: "自定义（手动填写 URL）" }],
                },
              ]}
            />
          </Form.Item>
          <Form.Item
            name="name"
            label="显示名称"
            rules={[{ required: true, message: "请填写名称" }]}
          >
            <Input placeholder="例如 DeepSeek Chat" />
          </Form.Item>
          <Form.Item
            name="baseModelUrl"
            label="Base URL"
            rules={[{ required: true, message: "请填写 Base URL" }]}
          >
            <Input
              placeholder="https://api.example.com/v1"
              readOnly={!isCustomPreset}
              variant={isCustomPreset ? undefined : "borderless"}
            />
          </Form.Item>
          <Form.Item
            name="modelName"
            label="模型 ID"
            rules={[{ required: true, message: "请填写模型 ID" }]}
          >
            <Input
              placeholder="model-name"
              readOnly={!isCustomPreset}
              variant={isCustomPreset ? undefined : "borderless"}
            />
          </Form.Item>
          <Form.Item
            name="apiKey"
            label={editing ? "API Key（留空不修改）" : "API Key"}
            rules={editing ? [] : [{ required: true, message: "请填写 API Key" }]}
          >
            <Input.Password placeholder="sk-..." autoComplete="new-password" />
          </Form.Item>
        </Form>
      </Modal>

      <Modal
        title={testingRow ? `测试模型 · ${testingRow.name}` : "测试模型"}
        open={testModalOpen}
        onCancel={() => setTestModalOpen(false)}
        footer={
          <Space>
            <Button onClick={() => setTestModalOpen(false)}>关闭</Button>
            <Button
              type="primary"
              icon={<ThunderboltOutlined />}
              loading={testing}
              onClick={() => runTest().catch(() => {})}
            >
              发送测试
            </Button>
          </Space>
        }
        destroyOnClose
        width={640}
      >
        {testingRow ? (
          <Typography.Paragraph type="secondary" style={{ marginTop: 0 }}>
            模型 ID：<Typography.Text code>{testingRow.modelName}</Typography.Text>
            <br />
            Base URL：<Typography.Text code>{testingRow.baseModelUrl}</Typography.Text>
          </Typography.Paragraph>
        ) : null}
        <Form
          form={testForm}
          layout="vertical"
          initialValues={{
            prompt: DEFAULT_TEST_PROMPT,
            thinkingMode: "default",
            maxTokens: 256,
          }}
        >
          <Form.Item name="prompt" label="测试提示词">
            <Input.TextArea rows={3} placeholder={DEFAULT_TEST_PROMPT} />
          </Form.Item>
          <Form.Item
            name="thinkingMode"
            label="Thinking"
            tooltip="默认=不传参由上游决定；开启/关闭对应通用 thinking 开关（DeepSeek / Qwen 等）"
          >
            <Select
              options={[
                { value: "default", label: "默认（上游决定）" },
                { value: "on", label: "开启" },
                { value: "off", label: "关闭" },
              ]}
            />
          </Form.Item>
          <Collapse
            ghost
            items={[
              {
                key: "advanced",
                label: "扩展参数（可选）",
                children: (
                  <>
                    <Form.Item name="temperature" label="Temperature (0–2)">
                      <InputNumber min={0} max={2} step={0.1} style={{ width: "100%" }} />
                    </Form.Item>
                    <Form.Item name="topP" label="Top P (0–1)">
                      <InputNumber min={0} max={1} step={0.05} style={{ width: "100%" }} />
                    </Form.Item>
                    <Form.Item name="maxTokens" label="Max tokens">
                      <InputNumber min={1} max={32768} step={1} style={{ width: "100%" }} />
                    </Form.Item>
                    <Form.Item name="frequencyPenalty" label="Frequency penalty (-2–2)">
                      <InputNumber min={-2} max={2} step={0.1} style={{ width: "100%" }} />
                    </Form.Item>
                    <Form.Item name="presencePenalty" label="Presence penalty (-2–2)">
                      <InputNumber min={-2} max={2} step={0.1} style={{ width: "100%" }} />
                    </Form.Item>
                    <Form.Item
                      name="reasoningEffort"
                      label="Reasoning effort"
                      tooltip="OpenAI 推理模型：low / medium / high"
                    >
                      <Select
                        allowClear
                        placeholder="不指定"
                        options={[
                          { value: "low", label: "low" },
                          { value: "medium", label: "medium" },
                          { value: "high", label: "high" },
                        ]}
                      />
                    </Form.Item>
                  </>
                ),
              },
            ]}
          />
        </Form>
        {testing ? (
          <div style={{ marginTop: 12 }}>
            <Spin tip="正在请求上游大模型…" />
          </div>
        ) : null}
        {testResult && !testing ? (
          <Alert
            style={{ marginTop: 12 }}
            type={testResult.ok ? "success" : "error"}
            showIcon
            message={
              testResult.ok
                ? `通过 · ${testResult.durationMs}ms`
                : `未通过 · ${testResult.status} · ${testResult.durationMs}ms`
            }
            description={
              <div>
                <div>
                  <Typography.Text type="secondary">上游：</Typography.Text>{" "}
                  <Typography.Text code>{testResult.upstreamUrl}</Typography.Text>
                </div>
                {testResult.thinkingEnabled !== undefined ? (
                  <div>
                    <Typography.Text type="secondary">Thinking：</Typography.Text>{" "}
                    {testResult.thinkingEnabled ? "开启" : "关闭"}
                  </div>
                ) : (
                  <div>
                    <Typography.Text type="secondary">Thinking：</Typography.Text> 默认
                  </div>
                )}
                {testResult.usage ? (
                  <div>
                    <Typography.Text type="secondary">Token：</Typography.Text>{" "}
                    in {testResult.usage.inputTokens} / out {testResult.usage.outputTokens} / total{" "}
                    {testResult.usage.totalTokens}
                  </div>
                ) : null}
                {testResult.thinkingText ? (
                  <div style={{ marginTop: 8 }}>
                    <Typography.Text type="secondary">Thinking 输出：</Typography.Text>
                    <Typography.Paragraph
                      code
                      style={{ marginBottom: 0, whiteSpace: "pre-wrap" }}
                    >
                      {testResult.thinkingText}
                    </Typography.Paragraph>
                  </div>
                ) : null}
                {testResult.responseText ? (
                  <div style={{ marginTop: 8 }}>
                    <Typography.Text type="secondary">回复：</Typography.Text>
                    <Typography.Paragraph
                      code
                      style={{ marginBottom: 0, whiteSpace: "pre-wrap" }}
                    >
                      {testResult.responseText}
                    </Typography.Paragraph>
                  </div>
                ) : null}
                {testResult.warnings.map((w) => (
                  <div key={w} style={{ marginTop: 4 }}>
                    <Typography.Text type="warning">{w}</Typography.Text>
                  </div>
                ))}
                {testResult.errors.map((e) => (
                  <div key={e} style={{ marginTop: 4 }}>
                    <Typography.Text type="danger">{e}</Typography.Text>
                  </div>
                ))}
                <Typography.Paragraph
                  type="secondary"
                  style={{ marginTop: 8, marginBottom: 0, fontSize: 12 }}
                >
                  {testResult.hint}
                </Typography.Paragraph>
              </div>
            }
          />
        ) : null}
      </Modal>
    </div>
  );
}
