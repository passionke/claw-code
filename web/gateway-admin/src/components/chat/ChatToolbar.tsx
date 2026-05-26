import { Button, Input, Space, Typography } from "antd";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import { useChatSession } from "../../context/ChatSessionContext";
import { isValidHttpUrl } from "../../utils/claudeTap";

export interface ChatToolbarProps {
  onNewSession: () => void;
  onHealth: (text: string) => void;
  onError: (text: string) => void;
}

/** store_id / org_id / claude-tap Live。Author: kejiqing */
export default function ChatToolbar({ onNewSession, onHealth, onError }: ChatToolbarProps) {
  const { gatewayBase } = useApp();
  const {
    storeId,
    setStoreId,
    orgId,
    setOrgId,
    tapLiveBase,
    refreshTapLive,
  } = useChatSession();

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
      <Space direction="vertical" size={4}>
        <Typography.Text type="secondary" style={{ fontSize: 12 }}>
          store_id（可选）
        </Typography.Text>
        <Input
          value={storeId}
          onChange={(e) => setStoreId(e.target.value)}
          placeholder="例 S20241007172800004204"
          autoComplete="off"
          style={{ width: 200 }}
        />
      </Space>
      <Space direction="vertical" size={4}>
        <Typography.Text type="secondary" style={{ fontSize: 12 }}>
          org_id（可选，保留空格）
        </Typography.Text>
        <Input
          value={orgId}
          onChange={(e) => setOrgId(e.target.value)}
          placeholder="与 store_id 二选一或同时填"
          autoComplete="off"
          style={{ width: 200 }}
        />
      </Space>
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
            {tapLiveBase}（无效，需重建 gateway 或修正 CLAW_GATEWAY_PUBLIC_BASE_URL）
          </Typography.Text>
        ) : (
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            —
          </Typography.Text>
        )}
      </Space>
      <Space>
        <Button onClick={onNewSession}>新会话</Button>
        <Button onClick={() => void runHealth()}>健康检查</Button>
      </Space>
    </>
  );
}
