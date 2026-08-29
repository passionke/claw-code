import { Alert, Button } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import type { ClusterBootstrapSnapshot } from "../types/globalSettings";

export default function BootstrapBanner() {
  const { gatewayBase } = useApp();
  const nav = useNavigate();
  const [snap, setSnap] = useState<ClusterBootstrapSnapshot | null>(null);

  const load = useCallback(async () => {
    if (!gatewayBase) return;
    try {
      const data = await proxyHttp<ClusterBootstrapSnapshot>(
        gatewayBase,
        "GET",
        "/v1/gateway/bootstrap/status"
      );
      setSnap(data);
    } catch {
      setSnap(null);
    }
  }, [gatewayBase]);

  useEffect(() => {
    void load();
    const t = window.setInterval(() => void load(), 15000);
    return () => clearInterval(t);
  }, [load]);

  if (!snap?.needsBootstrap) return null;

  return (
    <Alert
      type="warning"
      showIcon
      style={{ marginBottom: 16 }}
      message="首次集群引导未完成"
      description={
        snap.blockingReason
          ? `当前阻塞：${snap.blockingReason}。请在开发机构建 e2b 模板（clusterId=${snap.clusterId}），并在引导页配置 LLM。`
          : `clusterId=${snap.clusterId} — 请完成 LLM 与 e2b 模板引导。`
      }
      action={
        <Button size="small" type="primary" onClick={() => nav("/global/bootstrap")}>
          打开引导
        </Button>
      }
    />
  );
}
