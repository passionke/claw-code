"use client";

import { createContext, useCallback, useContext, useEffect, useState } from "react";
import { STORAGE_DS_ID, defaultDsId, readStoredDsId } from "@/lib/claw-config";
import { bootstrapProject, randomSessionId } from "@/lib/claw-conversation-bootstrap";
import {
  createSessionApi,
  notifyStoreUpdated,
  setActiveSessionApi,
} from "@/lib/claw-conversation-client";
import { projectIdFromDsId } from "@/lib/claw-conversation-types";

export type ClawUiContextValue = {
  dsId: number;
  threadId: string;
  setDsId: (n: number) => void;
  newSession: () => Promise<void>;
  selectSession: (sessionId: string) => Promise<void>;
  switching: boolean;
};

const ClawUiContext = createContext<ClawUiContextValue | null>(null);

export function useClawUi(): ClawUiContextValue {
  const ctx = useContext(ClawUiContext);
  if (!ctx) throw new Error("useClawUi outside ClawCopilotProvider");
  return ctx;
}

type Props = {
  children: React.ReactNode;
};

export function ClawHydratePlaceholder({ message = "Loading Claw…" }: { message?: string }) {
  return (
    <div className="claw-hydrate-placeholder" aria-busy="true" data-testid="claw-hydrate-placeholder">
      <p className="claw-hydrate-placeholder-text">{message}</p>
    </div>
  );
}

export function ClawCopilotProvider({ children }: Props) {
  const [ready, setReady] = useState(false);
  const [bootError, setBootError] = useState<string | null>(null);
  const [dsId, setDsIdState] = useState(() => defaultDsId());
  const [threadId, setThreadId] = useState<string | null>(null);
  const [switching, setSwitching] = useState(false);

  const activateForProject = useCallback(async (resolvedDs: number) => {
    const id = await bootstrapProject(resolvedDs);
    const projectId = projectIdFromDsId(resolvedDs);
    await setActiveSessionApi(projectId, id);
    setThreadId(id);
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const resolvedDs = readStoredDsId();
        if (cancelled) return;
        setDsIdState(resolvedDs);
        document.cookie = `claw_ds_id=${resolvedDs}; path=/; SameSite=Lax`;
        await activateForProject(resolvedDs);
        if (!cancelled) {
          setBootError(null);
          setReady(true);
        }
      } catch (e) {
        if (!cancelled) {
          setBootError(e instanceof Error ? e.message : String(e));
          setReady(false);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [activateForProject]);

  const setDsId = useCallback(
    (n: number) => {
      void (async () => {
        setDsIdState(n);
        localStorage.setItem(STORAGE_DS_ID, String(n));
        document.cookie = `claw_ds_id=${n}; path=/; SameSite=Lax`;
        setReady(false);
        setSwitching(true);
        try {
          await activateForProject(n);
          setBootError(null);
          setReady(true);
          notifyStoreUpdated();
        } catch (e) {
          setBootError(e instanceof Error ? e.message : String(e));
        } finally {
          setSwitching(false);
        }
      })();
    },
    [activateForProject],
  );

  const newSession = useCallback(async () => {
    const projectId = projectIdFromDsId(dsId);
    const id = randomSessionId();
    setSwitching(true);
    try {
      await createSessionApi(projectId, id);
      await setActiveSessionApi(projectId, id);
      setThreadId(id);
      notifyStoreUpdated();
    } finally {
      setSwitching(false);
    }
  }, [dsId]);

  const selectSession = useCallback(
    async (sessionId: string) => {
      if (sessionId === threadId) return;
      const projectId = projectIdFromDsId(dsId);
      setSwitching(true);
      try {
        setThreadId(sessionId);
        await setActiveSessionApi(projectId, sessionId);
      } finally {
        setSwitching(false);
      }
    },
    [dsId, threadId],
  );

  if (bootError) {
    return (
      <ClawHydratePlaceholder
        message={`对话库不可用：${bootError}。请启动 PostgreSQL 并设置 CLAW_WEB_DATABASE_URL。`}
      />
    );
  }

  if (!ready || !threadId) {
    return <ClawHydratePlaceholder />;
  }

  return (
    <ClawUiContext.Provider
      value={{ dsId, threadId, setDsId, newSession, selectSession, switching }}
    >
      {children}
    </ClawUiContext.Provider>
  );
}
