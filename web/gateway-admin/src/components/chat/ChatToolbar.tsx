import { TranslationOutlined } from "@ant-design/icons";
import { Button, Space, Typography } from "antd";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import { useChatSession } from "../../context/ChatSessionContext";
import { isValidHttpUrl } from "../../utils/claudeTap";

export interface ChatToolbarProps {
  onNewSession: () => void;
  onHealth: (text: string) => void;
  onError: (text: string) => void;
  onTranslateConversation?: () => void;
  translateDisabled?: boolean;
}

/** claude-tap Live + session actions. Author: kejiqing */
export default function ChatToolbar({
  onNewSession,
  onHealth,
  onError,
  onTranslateConversation,
  translateDisabled,
}: ChatToolbarProps) {
  const { gatewayBase } = useApp();
  const { tapLiveBase, refreshTapLive } = useChatSession();

  const runHealth = async () => {
    if (!gatewayBase) {
      onError("未选择网关");
      return;
    }
    try {
      const json = await proxyHttp(gatewayBase, "GET", "/healthz");
      onHealth(JSON.stringify(json, null, 2));
      await refreshTapLive();
    } catch (e) {
      onError(String((e as Error).message || e));
    }
  };

  return (
    <>
      <Space direction="vertical" size={4} style={{ minWidth: 200 }}>
        <Typography.Text type="secondary" style={{ fontSize: 12 }}>
          claude-tap Live
        </Typography.Text>
        {tapLiveBase && isValidHttpUrl(tapLiveBase) ? (
          <a href={tapLiveBase} target="_blank" rel="noopener noreferrer" style={{ fontSize: 12 }}>
            {tapLiveBase}
          </a>
        ) : tapLiveBase ? (
          <Typography.Text type="warning" style={{ fontSize: 12 }}>
            {tapLiveBase}（无效，请在「全局推理」保存 clawTap 主机与 Live 端口）
          </Typography.Text>
        ) : (
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            —
          </Typography.Text>
        )}
      </Space>
      <Space>
        <Button onClick={onNewSession}>新会话</Button>
        <Button
          icon={<TranslationOutlined />}
          disabled={translateDisabled}
          onClick={onTranslateConversation}
        >
          翻译中文
        </Button>
        <Button onClick={() => void runHealth()}>健康检查</Button>
      </Space>
    </>
  );
}
