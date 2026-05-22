import { Button, Input, Space, Spin, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import { putProjectConfigDraft } from "../utils/projectConfig";

const { TextArea } = Input;

type EffectivePromptResponse = {
  message?: string;
  promptSource?: string;
};

export default function PromptPage() {
  const { gatewayBase, dsId, projectConfig, refreshProjectConfig } = useApp();
  const [messageText, setMessageText] = useState("");
  const [promptSource, setPromptSource] = useState<string>("");
  const [loading, setLoading] = useState(false);
  const [pushing, setPushing] = useState(false);
  const [restoring, setRestoring] = useState(false);

  const loadPreview = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<EffectivePromptResponse>(
        gatewayBase,
        "GET",
        `/v1/project/prompt/${dsId}/effective`
      );
      setMessageText(r.message || "");
      setPromptSource(r.promptSource || "");
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

  const restoreDefault = async () => {
    const cfg = projectConfig ?? (await refreshProjectConfig());
    setRestoring(true);
    try {
      await putProjectConfigDraft(gatewayBase, dsId, cfg, { claudeMd: null });
      message.success("已恢复系统默认（已清空项目自定义系统提示词）");
      await refreshProjectConfig();
      await loadPreview();
    } finally {
      setRestoring(false);
    }
  };

  const sourceHint =
    promptSource === "user"
      ? "当前为项目自定义全文（CLAUDE.md 页保存的非空内容），不含系统默认前置段。"
      : "当前为系统默认（gateway_global_settings）+ 本项目 Rules / Skills 等拼装。";

  return (
    <div>
      <Typography.Title level={4}>系统提示词</Typography.Title>
      <Typography.Paragraph type="secondary">
        进入本页自动预览。系统默认模板存在数据库{" "}
        <Typography.Text code>gateway_global_settings.system_prompt_default</Typography.Text>
        （无 Admin 写接口，仅 DB 迁移更新）。在 <strong>CLAUDE.md</strong>{" "}
        页填写并保存非空内容后，将<strong>仅</strong>使用该自定义全文。
      </Typography.Paragraph>
      {promptSource ? (
        <Typography.Paragraph type="secondary" style={{ fontSize: 12 }}>
          {sourceHint}
        </Typography.Paragraph>
      ) : null}
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
        <Button
          loading={restoring}
          onClick={() => restoreDefault().catch((e) => message.error(String(e)))}
        >
          恢复默认
        </Button>
      </Space>
    </div>
  );
}
