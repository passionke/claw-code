import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import type { ClusterBootstrapSnapshot } from "../types/globalSettings";

const POLL_MS = 30_000;
const PUBLISH_POLL_MS = 3_000;

/** Cluster first-run gate — fetch + silent poll while bootstrap incomplete. Author: kejiqing */
export function useClusterBootstrap() {
  const { gatewayBase } = useApp();
  const [snap, setSnap] = useState<ClusterBootstrapSnapshot | null>(null);
  const [ready, setReady] = useState(false);
  const [refreshing, setRefreshing] = useState(false);

  const refresh = useCallback(
    async (silent = true): Promise<ClusterBootstrapSnapshot | null> => {
      if (!gatewayBase) return null;
      if (!silent) setRefreshing(true);
      try {
        const data = await proxyHttp<ClusterBootstrapSnapshot>(
          gatewayBase,
          "GET",
          "/v1/gateway/bootstrap/status"
        );
        setSnap(data);
        return data;
      } catch {
        // Keep last snapshot on poll failure (e.g. gateway restart) — do not fall through to Admin.
        if (!silent) setRefreshing(false);
        return null;
      } finally {
        if (!silent) setRefreshing(false);
      }
    },
    [gatewayBase]
  );

  useEffect(() => {
    let timer: number | undefined;

    const poll = async (silent: boolean) => {
      const data = await refresh(silent);
      setReady(true);
      // Init ack is PG completedAtMs. Component health must not reopen the wizard. Author: kejiqing
      const wizardOpen = data == null || data.completedAtMs == null;
      if (data && !wizardOpen) {
        if (timer !== undefined) {
          clearInterval(timer);
          timer = undefined;
        }
        return;
      }
      const interval =
        data?.publishJob?.phase === "running" ? PUBLISH_POLL_MS : POLL_MS;
      if (timer !== undefined) {
        clearInterval(timer);
        timer = undefined;
      }
      timer = window.setInterval(() => void poll(true), interval);
    };

    void poll(false);
    return () => {
      if (timer !== undefined) clearInterval(timer);
    };
  }, [refresh]);

  return {
    snap,
    ready,
    refreshing,
    // Wizard only when the DB ack is absent. Live phase failures stay on 核心组件. Author: kejiqing
    needsBootstrap: snap != null && snap.completedAtMs == null,
    refresh,
  };
}
