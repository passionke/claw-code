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

/** LLM polish only: `GET /v1/biz_advice_report_bak?task_id=…&stream=true`. Author: kejiqing */
export function usePolishReportStream(gatewayBase: string, taskId: string) {
  const [text, setText] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [error, setError] = useState(false);
  const esRef = useRef<EventSource | null>(null);
  const bufferRef = useRef("");
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

  const open = useCallback(() => {
    if (!gatewayBase || !taskId) return;
    close();
    endReasonRef.current = "idle";
    const path =
      `/v1/biz_advice_report_bak?task_id=${encodeURIComponent(taskId)}` + "&stream=true";
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
        "报告润色流错误";
      bufferRef.current = String(detail);
      setText(bufferRef.current);
      close();
    });

    es.onerror = () => {
      if (endReasonRef.current === "done") return;
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
  }, [gatewayBase, taskId, close]);

  useEffect(() => {
    endReasonRef.current = "idle";
    return () => close();
  }, [taskId, gatewayBase, close]);

  return { text, streaming, error, open, close };
}
