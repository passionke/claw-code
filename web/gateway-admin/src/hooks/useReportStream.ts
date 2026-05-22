import { useCallback, useEffect, useRef, useState } from "react";

function parseSseJson(raw: string): Record<string, unknown> | null {
  try {
    return JSON.parse(raw) as Record<string, unknown>;
  } catch {
    return null;
  }
}

function reportMessageFromDone(data: Record<string, unknown> | null): string {
  if (!data) return "";
  const rj = data.reportJson as { message?: string } | undefined;
  if (rj?.message) return String(rj.message);
  if (data.reportText) {
    const rt = parseSseJson(String(data.reportText));
    if (rt?.message) return String(rt.message);
    return String(data.reportText);
  }
  return "";
}

function proxySseTarget(gatewayBase: string, path: string): string {
  const u = `${gatewayBase.replace(/\/$/, "")}${path}`;
  return `/__proxy_sse__?target=${encodeURIComponent(u)}`;
}

/** Standard biz.report SSE (same as playground): start clears, delta appends, done finishes. Author: kejiqing */
export function useReportStream(
  gatewayBase: string,
  sessionId: string,
  turnId: string,
  dsId: number
) {
  const [text, setText] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [error, setError] = useState(false);
  const esRef = useRef<EventSource | null>(null);
  const bufferRef = useRef("");
  /** `done` = terminal `biz.report.done`; `error` = biz.report.error or EventSource drop. */
  const endReasonRef = useRef<"idle" | "done" | "error">("idle");

  const close = useCallback(() => {
    if (esRef.current) {
      try {
        esRef.current.close();
      } catch {
        /* ignore */
      }
      esRef.current = null;
    }
  }, []);

  /** Open (or reopen) SSE; gateway replays PG catch-up via `delta` after `start`. Author: kejiqing */
  const open = useCallback(() => {
    if (!gatewayBase) return;
    close();
    endReasonRef.current = "idle";
    const path =
      `/v1/biz_advice_report?sessionId=${encodeURIComponent(sessionId)}` +
      `&turnId=${encodeURIComponent(turnId)}` +
      `&dsId=${encodeURIComponent(String(dsId))}` +
      "&stream=true";
    setStreaming(true);
    setError(false);
    bufferRef.current = "";
    setText("");

    const es = new EventSource(proxySseTarget(gatewayBase, path));
    esRef.current = es;

    const appendDelta = (chunk: string) => {
      if (!chunk) return;
      bufferRef.current += chunk;
      setText(bufferRef.current);
    };

    const finish = (finalText: string) => {
      endReasonRef.current = "done";
      setStreaming(false);
      if (finalText) bufferRef.current = finalText;
      setText(bufferRef.current || "（无报告正文）");
      close();
    };

    // Worker/gateway replay always sends `start` after `open()` already cleared; do not wipe live buffer.
    es.addEventListener("biz.report.start", () => {});

    es.addEventListener("biz.report.delta", (ev) => {
      const data = parseSseJson(ev.data);
      if (data?.text) appendDelta(String(data.text));
    });

    es.addEventListener("biz.report.done", (ev) => {
      const data = parseSseJson(ev.data);
      finish(reportMessageFromDone(data) || bufferRef.current);
    });

    es.addEventListener("biz.report.error", (ev) => {
      endReasonRef.current = "error";
      setStreaming(false);
      setError(true);
      const data = parseSseJson(ev.data);
      const detail =
        (data && (data.detail || data.message || data.error)) ||
        ev.data ||
        "报告流错误";
      bufferRef.current = String(detail);
      setText(bufferRef.current);
      close();
    });

    es.onerror = () => {
      if (endReasonRef.current === "done") {
        return;
      }
      setStreaming(false);
      const buf = bufferRef.current;
      if (buf) {
        endReasonRef.current = "idle";
        setText(buf);
        close();
        return;
      }
      endReasonRef.current = "error";
      setError(true);
      setText("（报告连接中断）");
      close();
    };
  }, [gatewayBase, sessionId, turnId, dsId, close]);

  const canReconnect =
    endReasonRef.current !== "done";

  useEffect(() => {
    endReasonRef.current = "idle";
    return () => close();
  }, [sessionId, turnId, dsId, gatewayBase, close]);

  return { text, streaming, error, open, close, canReconnect };
};
