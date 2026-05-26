import { useCallback, useEffect, useRef, useState } from "react";

import {
  computeBizReportDensity,
  type BizReportDeltaRecord,
  type BizReportDensity,
} from "../utils/bizReportDensity";
import { extractSolveReportMessage } from "../utils/solveReportBody";

function parseSseJson(raw: string): Record<string, unknown> | null {
  try {
    return JSON.parse(raw) as Record<string, unknown>;
  } catch {
    return null;
  }
}

function proxySseTarget(gatewayBase: string, path: string): string {
  const u = `${gatewayBase.replace(/\/$/, "")}${path}`;
  return `/__proxy_sse__?target=${encodeURIComponent(u)}`;
}

function numField(data: Record<string, unknown> | null, key: string): number | undefined {
  const v = data?.[key];
  return typeof v === "number" && Number.isFinite(v) ? v : undefined;
}

/** Full report from `biz.report.done` (pool live or gateway snapshot). Author: kejiqing */
function reportTextFromDonePayload(data: Record<string, unknown> | null): string {
  if (!data) return "";
  const direct = data.reportText;
  if (typeof direct === "string" && direct.trim()) {
    return extractSolveReportMessage(direct);
  }
  const rj = data.reportJson ?? data.report_json;
  if (rj && typeof rj === "object" && rj !== null) {
    const msg = (rj as Record<string, unknown>).message;
    if (typeof msg === "string" && msg.trim()) {
      return extractSolveReportMessage(msg);
    }
  }
  return "";
}

export type BizReportStreamObs = {
  seq: number;
  maxTextLen: number;
  maxServerSameMsStreak: number;
  maxServerSameMsAt: number;
  maxClientSameMsStreak: number;
  maxClientSameMsAt: number;
  firstDeltaAtMs: number | null;
  lastDeltaAtMs: number | null;
};

declare global {
  interface Window {
    __bizReportObs?: BizReportStreamObs;
    __bizReportObsByTurn?: Record<string, BizReportStreamObs>;
    __bizReportDeltaLogByTurn?: Record<string, BizReportDeltaRecord[]>;
    __bizReportDensityByTurn?: Record<string, BizReportDensity>;
  }
}

/** Client-side burst log when many deltas share the same clientDeltaMs bucket. Author: kejiqing */
function logDeltaObs(
  chunk: string,
  data: Record<string, unknown> | null,
  clientDeltaMs: number,
  obs: BizReportStreamObs & {
    lastClientMs: number | null;
    sameClientMsStreak: number;
    lastServerMs: number | null;
    sameServerMsStreak: number;
  }
) {
  const serverDeltaMs = numField(data, "serverDeltaMs");
  const serverSeq = numField(data, "seq");
  const textLen = numField(data, "textLen") ?? chunk.length;

  obs.seq += 1;
  if (textLen > obs.maxTextLen) obs.maxTextLen = textLen;
  if (obs.firstDeltaAtMs == null) obs.firstDeltaAtMs = clientDeltaMs;
  obs.lastDeltaAtMs = clientDeltaMs;

  if (serverDeltaMs != null) {
    if (obs.lastServerMs === serverDeltaMs) {
      obs.sameServerMsStreak += 1;
    } else {
      if (obs.sameServerMsStreak + 1 > obs.maxServerSameMsStreak) {
        obs.maxServerSameMsStreak = obs.sameServerMsStreak + 1;
        obs.maxServerSameMsAt = obs.lastServerMs ?? serverDeltaMs;
      }
      obs.sameServerMsStreak = 0;
      obs.lastServerMs = serverDeltaMs;
    }
    if (obs.sameServerMsStreak >= 3) {
      console.warn("[biz-report-stream] server burst", {
        serverDeltaMs,
        sameServerStreak: obs.sameServerMsStreak + 1,
        seq: serverSeq ?? obs.seq,
      });
    }
  }

  if (obs.lastClientMs === clientDeltaMs) {
    obs.sameClientMsStreak += 1;
  } else {
    if (obs.sameClientMsStreak + 1 > obs.maxClientSameMsStreak) {
      obs.maxClientSameMsStreak = obs.sameClientMsStreak + 1;
      obs.maxClientSameMsAt = obs.lastClientMs ?? clientDeltaMs;
    }
    if (obs.sameClientMsStreak >= 5) {
      console.info("[biz-report-stream] client burst", {
        clientDeltaMs: obs.lastClientMs,
        count: obs.sameClientMsStreak + 1,
        serverDeltaMs: obs.lastServerMs,
      });
    }
    obs.sameClientMsStreak = 0;
    obs.lastClientMs = clientDeltaMs;
  }

  if (textLen >= 200) {
    console.warn("[biz-report-stream] large delta", {
      seq: serverSeq ?? obs.seq,
      textLen,
      serverDeltaMs,
      clientDeltaMs,
    });
  }

  console.info("[biz-report-stream] delta", {
    seq: serverSeq ?? obs.seq,
    textLen,
    serverDeltaMs,
    clientDeltaMs,
    sameServerStreak: obs.sameServerMsStreak,
    sameClientStreak: obs.sameClientMsStreak,
    skewMs: serverDeltaMs != null ? clientDeltaMs - serverDeltaMs : undefined,
  });
}

/** Live report: one EventSource, append `text` deltas (rAF batch per frame). Author: kejiqing */
export function useBizReportStream(
  gatewayBase: string,
  sessionId: string,
  turnId: string,
  dsId: number
) {
  const [text, setText] = useState("");
  const [live, setLive] = useState(false);
  const esRef = useRef<EventSource | null>(null);
  const closedRef = useRef(false);
  const pendingRef = useRef("");
  const rafRef = useRef<number | null>(null);
  const settledPromiseRef = useRef<Promise<void> | null>(null);
  const settledResolveRef = useRef<(() => void) | null>(null);

  const applyAuthoritativeReport = useCallback(
    (raw: string) => {
      const full = extractSolveReportMessage(raw);
      if (!full) return;
      setText((prev) => (full.length >= prev.length ? full : prev));
    },
    []
  );

  const flushPending = useCallback(() => {
    if (!pendingRef.current) return;
    const chunk = pendingRef.current;
    const chars = chunk.length;
    pendingRef.current = "";
    console.info("[biz-report-stream] raf flush", { chars });
    setText((prev) => prev + chunk);
  }, []);

  const scheduleFlush = useCallback(() => {
    if (rafRef.current != null) return;
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null;
      flushPending();
    });
  }, [flushPending]);

  const close = useCallback(() => {
    closedRef.current = true;
    if (rafRef.current != null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
    flushPending();
    if (esRef.current) {
      try {
        esRef.current.close();
      } catch {
        /* ignore */
      }
      esRef.current = null;
    }
    setLive(false);
    settledResolveRef.current?.();
    settledResolveRef.current = null;
  }, [flushPending]);

  const waitForSettled = useCallback((timeoutMs: number) => {
    const p = settledPromiseRef.current ?? Promise.resolve();
    const t = Math.max(0, timeoutMs);
    return Promise.race([
      p,
      new Promise<void>((resolve) => {
        setTimeout(resolve, t);
      }),
    ]);
  }, []);

  const open = useCallback(() => {
    if (!gatewayBase || !sessionId || !turnId || !dsId) return;
    if (esRef.current) {
      closedRef.current = true;
      try {
        esRef.current.close();
      } catch {
        /* ignore */
      }
      esRef.current = null;
    }
    closedRef.current = false;
    settledPromiseRef.current = new Promise<void>((resolve) => {
      settledResolveRef.current = resolve;
    });
    pendingRef.current = "";
    setText("");
    setLive(true);

    const t0 = performance.now();
    const obs: BizReportStreamObs & {
      lastClientMs: number | null;
      sameClientMsStreak: number;
      lastServerMs: number | null;
      sameServerMsStreak: number;
    } = {
      seq: 0,
      maxTextLen: 0,
      maxServerSameMsStreak: 0,
      maxServerSameMsAt: 0,
      maxClientSameMsStreak: 0,
      maxClientSameMsAt: 0,
      firstDeltaAtMs: null,
      lastDeltaAtMs: null,
      lastClientMs: null,
      sameClientMsStreak: 0,
      lastServerMs: null,
      sameServerMsStreak: 0,
    };
    window.__bizReportObs = obs;
    window.__bizReportObsByTurn = window.__bizReportObsByTurn ?? {};
    window.__bizReportObsByTurn[turnId] = obs;
    const deltaLog: BizReportDeltaRecord[] = [];
    window.__bizReportDeltaLogByTurn = window.__bizReportDeltaLogByTurn ?? {};
    window.__bizReportDeltaLogByTurn[turnId] = deltaLog;

    const q = new URLSearchParams({
      sessionId,
      turnId,
      dsId: String(dsId),
      stream: "true",
    });
    const es = new EventSource(
      proxySseTarget(gatewayBase, `/v1/biz_advice_report?${q.toString()}`)
    );
    esRef.current = es;

    es.addEventListener("biz.report.start", (ev) => {
      const data = parseSseJson(ev.data);
      console.info("[biz-report-stream] start", {
        taskId: data?.taskId,
        streamStartedAtMs: data?.streamStartedAtMs,
        turnId,
        sessionId,
      });
    });

    es.addEventListener("biz.report.delta", (ev) => {
      const data = parseSseJson(ev.data);
      const chunk = data?.text != null ? String(data.text) : "";
      if (!chunk) return;
      const clientDeltaMs = Math.round(performance.now() - t0);
      logDeltaObs(chunk, data, clientDeltaMs, obs);
      deltaLog.push({
        seq: numField(data, "seq") ?? obs.seq,
        serverDeltaMs: numField(data, "serverDeltaMs"),
        clientDeltaMs,
        textLen: numField(data, "textLen") ?? chunk.length,
      });
      pendingRef.current += chunk;
      scheduleFlush();
    });

    es.addEventListener("biz.report.done", (ev) => {
      const data = parseSseJson(ev.data);
      flushPending();
      const doneBody = reportTextFromDonePayload(data);
      if (doneBody) {
        applyAuthoritativeReport(doneBody);
      }
      if (obs.sameServerMsStreak + 1 > obs.maxServerSameMsStreak) {
        obs.maxServerSameMsStreak = obs.sameServerMsStreak + 1;
        obs.maxServerSameMsAt = obs.lastServerMs ?? 0;
      }
      if (obs.sameClientMsStreak + 1 > obs.maxClientSameMsStreak) {
        obs.maxClientSameMsStreak = obs.sameClientMsStreak + 1;
        obs.maxClientSameMsAt = obs.lastClientMs ?? 0;
      }
      const summary = {
        clientDeltas: obs.seq,
        maxTextLen: obs.maxTextLen,
        maxServerSameMsStreak: obs.maxServerSameMsStreak,
        maxServerSameMsAt: obs.maxServerSameMsAt,
        maxClientSameMsStreak: obs.maxClientSameMsStreak,
        maxClientSameMsAt: obs.maxClientSameMsAt,
        firstDeltaAtMs: obs.firstDeltaAtMs,
        lastDeltaAtMs: obs.lastDeltaAtMs,
        deltaCount: numField(data, "deltaCount"),
        streamDurationMs: numField(data, "streamDurationMs"),
        clientDurationMs: Math.round(performance.now() - t0),
      };
      const density = computeBizReportDensity(deltaLog, obs.maxClientSameMsStreak);
      window.__bizReportDensityByTurn = window.__bizReportDensityByTurn ?? {};
      window.__bizReportDensityByTurn[turnId] = density;
      console.info("[biz-report-stream] density", { turnId, ...density });
      console.info("[biz-report-stream] done", summary);
      window.__bizReportObs = obs;
      window.__bizReportObsByTurn = window.__bizReportObsByTurn ?? {};
      window.__bizReportObsByTurn[turnId] = obs;
      closedRef.current = true;
      if (esRef.current) {
        try {
          esRef.current.close();
        } catch {
          /* ignore */
        }
        esRef.current = null;
      }
      setLive(false);
      settledResolveRef.current?.();
      settledResolveRef.current = null;
    });

    es.onerror = () => {
      if (closedRef.current) return;
      flushPending();
      console.warn("[biz-report-stream] error", {
        clientDeltas: obs.seq,
        clientDurationMs: Math.round(performance.now() - t0),
      });
      closedRef.current = true;
      if (esRef.current) {
        try {
          esRef.current.close();
        } catch {
          /* ignore */
        }
        esRef.current = null;
      }
      setLive(false);
      settledResolveRef.current?.();
      settledResolveRef.current = null;
    };
  }, [
    gatewayBase,
    sessionId,
    turnId,
    dsId,
    close,
    flushPending,
    scheduleFlush,
    applyAuthoritativeReport,
  ]);

  useEffect(() => () => close(), [close]);

  return { text, live, open, close, waitForSettled, reconcileReport: applyAuthoritativeReport };
}
