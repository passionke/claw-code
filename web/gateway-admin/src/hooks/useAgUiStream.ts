/** Subscribe to Gateway AG-UI SSE for mid-run process disclosure. Author: kejiqing */
import { useCallback, useEffect, useRef, useState } from "react";

export type AgUiProcessStep = {
  id: string;
  kind: string;
  title: string;
  status: string;
  durationMs?: number | null;
  summary?: string;
};

export type AgUiA2uiSurface = {
  version?: string;
  catalogId?: string;
  surfaceId?: string;
  components?: Array<Record<string, unknown>>;
};

function proxySseTarget(gatewayBase: string, path: string): string {
  const u = `${gatewayBase.replace(/\/$/, "")}${path}`;
  return `/__proxy_sse__?target=${encodeURIComponent(u)}`;
}

function parseStepsFromA2ui(a2ui: AgUiA2uiSurface | null): AgUiProcessStep[] {
  if (!a2ui || a2ui.catalogId !== "claw-process/v1") return [];
  const comps = a2ui.components ?? [];
  return comps
    .filter((c) => c.component === "ProcessStep")
    .map((c) => ({
      id: String(c.id ?? ""),
      kind: String(c.kind ?? "tool"),
      title: String(c.title ?? ""),
      status: String(c.status ?? "running"),
      durationMs:
        typeof c.durationMs === "number"
          ? c.durationMs
          : c.durationMs == null
            ? null
            : Number(c.durationMs),
      summary: c.summary != null ? String(c.summary) : "",
    }));
}

/** Live AG-UI consumer: process steps + optional ask a2ui. Author: kejiqing */
export function useAgUiStream(
  gatewayBase: string,
  sessionId: string,
  turnId: string,
  projId: number,
  enabled: boolean
) {
  const [steps, setSteps] = useState<AgUiProcessStep[]>([]);
  const [processA2ui, setProcessA2ui] = useState<AgUiA2uiSurface | null>(null);
  const [askA2ui, setAskA2ui] = useState<AgUiA2uiSurface | null>(null);
  const [running, setRunning] = useState(false);
  const esRef = useRef<EventSource | null>(null);

  const close = useCallback(() => {
    esRef.current?.close();
    esRef.current = null;
    setRunning(false);
  }, []);

  const open = useCallback(() => {
    if (!gatewayBase || !sessionId || !turnId || !enabled) return;
    close();
    const path =
      `/v1/ag-ui/runs?sessionId=${encodeURIComponent(sessionId)}` +
      `&turnId=${encodeURIComponent(turnId)}` +
      `&projId=${encodeURIComponent(String(projId))}`;
    const es = new EventSource(proxySseTarget(gatewayBase, path));
    esRef.current = es;
    setRunning(true);
    es.onmessage = (ev) => {
      let data: Record<string, unknown> | null = null;
      try {
        data = JSON.parse(ev.data) as Record<string, unknown>;
      } catch {
        return;
      }
      const type = String(data.type ?? "");
      if (type === "RUN_STARTED") {
        setRunning(true);
        return;
      }
      if (type === "RUN_FINISHED" || type === "RUN_ERROR") {
        setRunning(false);
        es.close();
        esRef.current = null;
        return;
      }
      if (type === "CUSTOM" && data.name === "a2ui") {
        const value = (data.value ?? null) as AgUiA2uiSurface | null;
        if (value?.catalogId === "claw-process/v1") {
          setProcessA2ui(value);
          setSteps(parseStepsFromA2ui(value));
        } else if (value?.catalogId === "claw-ask/v1") {
          setAskA2ui(value);
        }
        return;
      }
      if (type === "CUSTOM" && data.name === "a2ui.cleared") {
        setAskA2ui(null);
      }
    };
    es.onerror = () => {
      /* browser auto-reconnects; leave open while turn may still be running */
    };
  }, [gatewayBase, sessionId, turnId, projId, enabled, close]);

  useEffect(() => {
    if (!enabled) {
      close();
      return;
    }
    open();
    return () => close();
  }, [enabled, open, close]);

  return { steps, processA2ui, askA2ui, running, open, close };
}
