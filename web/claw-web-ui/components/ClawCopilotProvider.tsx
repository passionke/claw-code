"use client";

import { CopilotKit } from "@copilotkit/react-core";
import "@copilotkit/react-ui/styles.css";
import { createContext, useCallback, useContext, useEffect, useState } from "react";
import { CLAW_AGENT_ID, STORAGE_DS_ID, STORAGE_THREAD_ID } from "@/lib/claw-config";

function randomThreadId(): string {
  if (typeof crypto !== "undefined" && crypto.randomUUID) {
    return crypto.randomUUID();
  }
  return `thread-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

type ClawUiContextValue = {
  dsId: number;
  threadId: string | null;
  setDsId: (n: number) => void;
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

export function ClawCopilotProvider({ children }: Props) {
  const [dsId, setDsIdState] = useState(1);
  const [threadId, setThreadId] = useState<string | null>(null);

  useEffect(() => {
    let id = localStorage.getItem(STORAGE_THREAD_ID);
    if (!id) {
      id = randomThreadId();
      localStorage.setItem(STORAGE_THREAD_ID, id);
    }
    setThreadId(id);

    const storedDs = localStorage.getItem(STORAGE_DS_ID);
    if (storedDs) {
      const n = Number.parseInt(storedDs, 10);
      if (Number.isFinite(n) && n > 0) setDsIdState(n);
    }
  }, []);

  const setDsId = useCallback((n: number) => {
    setDsIdState(n);
    localStorage.setItem(STORAGE_DS_ID, String(n));
    document.cookie = `claw_ds_id=${n}; path=/; SameSite=Lax`;
  }, []);

  if (!threadId) {
    return null;
  }

  return (
    <ClawUiContext.Provider value={{ dsId, threadId, setDsId }}>
      <CopilotKit
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
