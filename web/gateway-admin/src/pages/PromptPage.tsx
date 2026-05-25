import { Button, Input, Space, Spin, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import EditorLengthHint from "../components/EditorLengthHint";

const { TextArea } = Input;

type EffectivePromptResponse = {
  message?: string;
  promptSource?: string;
};

export default function PromptPage() {
  const { gatewayBase, dsId, projectConfig } = useApp();
  const [messageText, setMessageText] = useState("");
  const [loading, setLoading] = useState(false);
  const [pushing, setPushing] = useState(false);

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

  return (
    <div>
      <Typography.Title level={4}>系统提示词（当前生效）</Typography.Title>
      {projectConfig?.draftOpen ? (
        <Typography.Paragraph type="secondary" style={{ marginBottom: 8 }}>
          存在未提交草稿时，此处仍预览<strong>已生效正式版</strong>物化后的 system prompt，不是草稿内容。
        </Typography.Paragraph>
      ) : null}
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
