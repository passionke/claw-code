import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import type { ClusterBootstrapSnapshot } from "../types/globalSettings";

const POLL_MS = 30_000;

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
      if (data && !data.needsBootstrap) {
        if (timer !== undefined) {
          clearInterval(timer);
          timer = undefined;
        }
        return;
      }
      if (timer === undefined) {
        timer = window.setInterval(() => void poll(true), POLL_MS);
      }
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
    needsBootstrap: snap?.needsBootstrap === true,
    refresh,
  };
}
