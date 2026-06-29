import { TranslationOutlined } from "@ant-design/icons";
import { Button, Space } from "antd";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";

export interface ChatToolbarProps {
  onNewSession: () => void;
  onHealth: (text: string) => void;
  onError: (text: string) => void;
  onTranslateConversation?: () => void;
  translateDisabled?: boolean;
}

/** Session toolbar actions. Author: kejiqing */
export default function ChatToolbar({
  onNewSession,
  onHealth,
  onError,
  onTranslateConversation,
  translateDisabled,
}: ChatToolbarProps) {
  const { gatewayBase } = useApp();

  const runHealth = async () => {
    if (!gatewayBase) {
      onError("未选择网关");
      return;
    }
    try {
      const json = await proxyHttp(gatewayBase, "GET", "/healthz");
      onHealth(JSON.stringify(json, null, 2));
    } catch (e) {
      onError(String((e as Error).message || e));
    }
  };

  return (
    <>
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
