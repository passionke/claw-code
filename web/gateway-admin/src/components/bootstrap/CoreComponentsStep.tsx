import { Alert, Button, Space, Tag, Typography, message } from "antd";
import { useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type { BootstrapEnsureCoreResponse, ClusterBootstrapSnapshot } from "../../types/globalSettings";

type Props = {
  snap: ClusterBootstrapSnapshot;
  onRefresh: () => Promise<ClusterBootstrapSnapshot | null>;
  onNext: () => void;
};

function onlineTag(online?: boolean, label = "组件") {
  if (online === undefined) return <Tag>未知</Tag>;
  return <Tag color={online ? "success" : "warning"}>{online ? `${label} 在线` : `${label} 未就绪`}</Tag>;
}

/** Step 4: ensure nas-api + observe singletons. Author: kejiqing */
export default function CoreComponentsStep({ snap, onRefresh, onNext }: Props) {
  const { gatewayBase } = useApp();
  const [ensuring, setEnsuring] = useState(false);

  const singletonsOk = snap.phases.find((p) => p.phase === "e2b_singletons")?.complete;
  const nas = snap.singletons?.nasApi;
  const observe = snap.singletons?.observe;

  const ensureCore = async () => {
    if (!gatewayBase) return;
    setEnsuring(true);
    try {
      const resp = await proxyHttp<BootstrapEnsureCoreResponse>(
        gatewayBase,
        "POST",
        "/v1/gateway/bootstrap/ensure-core"
      );
      if (resp.ok) message.success("核心组件已就绪");
      else message.warning(resp.message ?? "尚未完全就绪");
      await onRefresh();
    } catch (e) {
      message.error(e instanceof Error ? e.message : "ensure-core 失败");
    } finally {
      setEnsuring(false);
    }
  };

  return (
    <Space direction="vertical" size="middle" style={{ width: "100%" }}>
      <Typography.Paragraph type="secondary">
        拉起 e2b observe / nas-api 单例。e2b 宿主机 NAS/traffic 在 e2bserver deploy.toml 配置。
      </Typography.Paragraph>

      <Space wrap>
        {onlineTag(nas?.online, "nas-api")}
        {onlineTag(observe?.healthy, "observe")}
      </Space>

      {snap.blockingReason && !singletonsOk ? (
        <Alert type="warning" showIcon message={snap.blockingReason} />
      ) : null}

      <Space wrap>
        <Button type="primary" loading={ensuring} onClick={() => void ensureCore()}>
          确保核心组件
        </Button>
        <Button onClick={() => void onRefresh()}>刷新状态</Button>
        {singletonsOk ? (
          <Button type="primary" onClick={onNext}>
            下一步
          </Button>
        ) : null}
      </Space>
    </Space>
  );
}
