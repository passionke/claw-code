"use client";

import { CopilotKit } from "@copilotkit/react-core";
import "@copilotkit/react-ui/styles.css";
import "@/app/claw-copilot.css";
import { createContext, useCallback, useContext, useEffect, useState } from "react";
import { CLAW_AGENT_ID, STORAGE_DS_ID, defaultDsId, readStoredDsId } from "@/lib/claw-config";
import { bootstrapProject, randomSessionId } from "@/lib/claw-conversation-bootstrap";
import {
  createSessionApi,
  notifyStoreUpdated,
  setActiveSessionApi,
} from "@/lib/claw-conversation-client";
import { projectIdFromDsId } from "@/lib/claw-conversation-types";

type ClawUiContextValue = {
  dsId: number;
  threadId: string | null;
  setDsId: (n: number) => void;
  newSession: () => void;
  selectSession: (sessionId: string) => void;
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

  const activateForProject = useCallback(async (resolvedDs: number) => {
    const id = await bootstrapProject(resolvedDs);
    setThreadId(id);
  }, []);

  useEffect(() => {
    let cancelled = false;
    (async () => {
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
        try {
          await activateForProject(n);
          setBootError(null);
          setReady(true);
          notifyStoreUpdated();
        } catch (e) {
          setBootError(e instanceof Error ? e.message : String(e));
        }
      })();
    },
    [activateForProject],
  );

  const newSession = useCallback(() => {
    void (async () => {
      const projectId = projectIdFromDsId(dsId);
      const record = await createSessionApi(projectId, randomSessionId());
      setThreadId(record.sessionId);
      notifyStoreUpdated();
    })();
  }, [dsId]);

  const selectSession = useCallback(
    (sessionId: string) => {
      void (async () => {
        const projectId = projectIdFromDsId(dsId);
        await setActiveSessionApi(projectId, sessionId);
        setThreadId(sessionId);
        notifyStoreUpdated();
      })();
    },
    [dsId],
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
    <ClawUiContext.Provider value={{ dsId, threadId, setDsId, newSession, selectSession }}>
      <CopilotKit
        key={threadId}
        threadId={threadId}
        runtimeUrl="/api/copilotkit"
        agent={CLAW_AGENT_ID}
        headers={{
          "x-claw-ds-id": String(dsId),
          "x-claw-thread-id": threadId,
        }}
        properties={{
          forwardedProps: { dsId },
        }}
      >
        {children}
      </CopilotKit>
    </ClawUiContext.Provider>
  );
}
