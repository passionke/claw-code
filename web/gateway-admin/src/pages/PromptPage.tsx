import { Button, Input, InputNumber, Space, Spin, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import EditorLengthHint from "../components/EditorLengthHint";
import { putProjectConfigDraft } from "../utils/projectConfig";
import type { PromptLimitsJson } from "../types/project";

const { TextArea } = Input;

/** Runtime defaults when `promptLimitsJson` is empty. Author: kejiqing */
export const DEFAULT_INSTRUCTION_FILE_MAX_CHARS = 8000;
export const DEFAULT_INSTRUCTION_TOTAL_MAX_CHARS = 24000;

type EffectivePromptResponse = {
  message?: string;
  promptSource?: string;
};

function limitsFromConfig(
  raw: PromptLimitsJson | undefined
): { fileMax: number; totalMax: number } {
  return {
    fileMax: raw?.instructionFileMaxChars ?? DEFAULT_INSTRUCTION_FILE_MAX_CHARS,
    totalMax: raw?.instructionTotalMaxChars ?? DEFAULT_INSTRUCTION_TOTAL_MAX_CHARS,
  };
}

export default function PromptPage() {
  const { gatewayBase, dsId, projectConfig, refreshProjectConfig, applyProjectConfig } =
    useApp();
  const [messageText, setMessageText] = useState("");
  const [loading, setLoading] = useState(false);
  const [pushing, setPushing] = useState(false);
  const [savingLimits, setSavingLimits] = useState(false);
  const [fileMax, setFileMax] = useState(DEFAULT_INSTRUCTION_FILE_MAX_CHARS);
  const [totalMax, setTotalMax] = useState(DEFAULT_INSTRUCTION_TOTAL_MAX_CHARS);

  useEffect(() => {
    const { fileMax: f, totalMax: t } = limitsFromConfig(projectConfig?.promptLimitsJson);
    setFileMax(f);
    setTotalMax(t);
  }, [projectConfig?.promptLimitsJson, projectConfig?.updatedAtMs]);

  const loadPreview = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<EffectivePromptResponse>(
        gatewayBase,
        "GET",
        `/v1/project/prompt/${dsId}/effective`
      );
      setMessageText(r.message || "");
    } finally {
      setLoading(false);
    }
  }, [gatewayBase, dsId]);

  useEffect(() => {
    loadPreview().catch((e) => message.error(String(e)));
  }, [loadPreview, projectConfig?.contentRev, projectConfig?.updatedAtMs]);

  const refreshRuntime = async () => {
    setPushing(true);
    try {
      await proxyHttp<EffectivePromptResponse>(
        gatewayBase,
        "POST",
        `/v1/project/prompt/${dsId}/effective`
      );
      message.success("已刷新到运行时");
      await loadPreview();
    } finally {
      setPushing(false);
    }
  };

  const saveLimits = async () => {
    if (!projectConfig) {
      message.error("未加载项目配置");
      return;
    }
    if (fileMax < 1 || totalMax < 1) {
      message.error("长度必须为正整数");
      return;
    }
    setSavingLimits(true);
    try {
      const next = await putProjectConfigDraft(gatewayBase, dsId, projectConfig, {
        promptLimitsJson: {
          instructionFileMaxChars: fileMax,
          instructionTotalMaxChars: totalMax,
        },
      });
      applyProjectConfig(next);
      message.success("已保存到草稿（固化并生效后 solve 使用）");
    } catch (e) {
      message.error(String(e));
    } finally {
      setSavingLimits(false);
    }
  };

  return (
    <div>
      <Typography.Title level={4}>系统提示词（当前生效）</Typography.Title>
      {projectConfig?.draftOpen ? (
        <Typography.Paragraph type="secondary" style={{ marginBottom: 8 }}>
          存在未提交草稿时，此处仍预览<strong>已生效正式版</strong>物化后的 system prompt，不是草稿内容。
        </Typography.Paragraph>
      ) : null}

      <Typography.Title level={5} style={{ marginTop: 16 }}>
        指令长度预算
      </Typography.Title>
      <Typography.Paragraph type="secondary" style={{ marginBottom: 8 }}>
        写入 <Typography.Text code>project_config.prompt_limits_json</Typography.Text>，物化到{" "}
        <Typography.Text code>.claw/settings.json</Typography.Text>。单文件上限作用于每个{" "}
        <Typography.Text code>CLAUDE.md</Typography.Text> / rule；总上限分别作用于{" "}
        <Typography.Text code># Claude instructions</Typography.Text> 与{" "}
        <Typography.Text code># Project rules</Typography.Text> 段（各 24k 默认）。
      </Typography.Paragraph>
      <Space wrap style={{ marginBottom: 16 }}>
        <span>
          单文件上限（字符）
          <InputNumber
            min={1}
            max={1_000_000}
            value={fileMax}
            onChange={(v) => setFileMax(v ?? DEFAULT_INSTRUCTION_FILE_MAX_CHARS)}
            style={{ width: 140, marginLeft: 8 }}
          />
        </span>
        <span>
          段内合计上限（字符）
          <InputNumber
            min={1}
            max={1_000_000}
            value={totalMax}
            onChange={(v) => setTotalMax(v ?? DEFAULT_INSTRUCTION_TOTAL_MAX_CHARS)}
            style={{ width: 140, marginLeft: 8 }}
          />
        </span>
        <Button loading={savingLimits} onClick={() => void saveLimits()}>
          保存长度配置
        </Button>
        <Button type="link" onClick={() => void refreshProjectConfig()}>
          重新加载配置
        </Button>
      </Space>

      <EditorLengthHint text={messageText} label="运行时系统提示词预览" />
      <Spin spinning={loading}>
        <TextArea
          rows={18}
          readOnly
          value={messageText}
          placeholder={loading ? "加载中…" : "暂无内容"}
        />
      </Spin>
      <Space style={{ marginTop: 12 }} wrap>
        <Button
          type="primary"
          loading={pushing}
          onClick={() => refreshRuntime().catch((e) => message.error(String(e)))}
        >
          刷新到运行时
        </Button>
      </Space>
    </div>
  );
}
