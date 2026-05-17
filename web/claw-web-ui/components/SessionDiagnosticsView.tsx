"use client";

import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { formatTimelineTime, type TimelineEntry } from "@/lib/build-session-timeline";
import { readStoredDsId } from "@/lib/claw-config";
import { fetchConversationIndex } from "@/lib/claw-conversation-client";
import { projectIdFromDsId } from "@/lib/claw-conversation-types";

type DiagnosticsPayload = {
  sessionId: string;
  dsId: number;
  taskStatus: string;
  errorStrip: string | null;
  timeline: TimelineEntry[];
  error?: string;
  detail?: string;
};

type Props = {
  sessionId: string;
  dsIdParam?: string | null;
};

/** Standalone session timeline (gateway-correlated). Author: kejiqing */
export function SessionDiagnosticsView({ sessionId, dsIdParam }: Props) {
  const [data, setData] = useState<DiagnosticsPayload | null>(null);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);
  const [errDetail, setErrDetail] = useState<string | null>(null);
  const [pgActiveSession, setPgActiveSession] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setErr(null);
    setErrDetail(null);
    const dsId =
      dsIdParam && Number.isFinite(Number.parseInt(dsIdParam, 10))
        ? Number.parseInt(dsIdParam, 10)
        : readStoredDsId();
    try {
      const res = await fetch(
        `/api/claw/sessions/${encodeURIComponent(sessionId)}/diagnostics?dsId=${dsId}`,
        { cache: "no-store" },
      );
      const body = (await res.json()) as DiagnosticsPayload & { error?: string };
      if (!res.ok) {
        setErr(body.error ?? `HTTP ${res.status}`);
        setErrDetail(body.detail ?? null);
        setData(null);
        return;
      }
      setData(body);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
      setData(null);
    } finally {
      setLoading(false);
    }
  }, [sessionId, dsIdParam]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!err) {
      setPgActiveSession(null);
      return;
    }
    const projectId = projectIdFromDsId(readStoredDsId());
    void fetchConversationIndex(projectId)
      .then((d) => setPgActiveSession(d.activeSessionId))
      .catch(() => setPgActiveSession(null));
  }, [err, sessionId]);

  const status = data?.taskStatus ?? "…";
  const pillClass =
    status === "failed"
      ? "fail"
      : status === "succeeded"
        ? "ok"
        : "idle";

  const copyId = useCallback(() => {
    void navigator.clipboard.writeText(sessionId);
  }, [sessionId]);

  return (
    <div className="claw-diag-root">
      <header className="claw-diag-top">
        <Link className="claw-diag-back" href="/">
          ← Claw
        </Link>
        <h1>Session diagnostics</h1>
        <span className="claw-diag-sid" title={sessionId}>
          {sessionId}
        </span>
        {!loading && data && (
          <span className={`claw-diag-pill ${pillClass}`}>{status}</span>
        )}
        <button type="button" className="claw-diag-btn" onClick={() => void copyId()}>
          Copy id
        </button>
        <button type="button" className="claw-diag-btn" onClick={() => void load()}>
          刷新
        </button>
      </header>

      {data?.errorStrip && (
        <p className="claw-diag-err-strip">
          <code>{data.errorStrip}</code>
        </p>
      )}

      {loading && <p className="claw-diag-loading">加载中…</p>}
      {!loading && err && (
        <div className="claw-diag-empty">
          <p>
            <strong>{err}</strong>
          </p>
          {errDetail && <p className="claw-diag-empty-detail">{errDetail}</p>}
          {pgActiveSession && pgActiveSession !== sessionId ? (
            <p className="claw-diag-empty-detail">
              当前工程活跃会话为{" "}
              <Link
                href={`/session/${encodeURIComponent(pgActiveSession)}?dsId=${readStoredDsId()}`}
              >
                {pgActiveSession}
              </Link>
              ，可点此查看诊断。
            </p>
          ) : (
            <p className="claw-diag-empty-detail">
              请从主页侧栏 <strong>claw-session-id</strong> 旁点击「诊断」。
            </p>
          )}
        </div>
      )}
      {!loading && !err && data && data.timeline.length === 0 && (
        <p className="claw-diag-empty">暂无事件（会话存在但尚无 progress / tap / trace）。</p>
      )}
      {!loading && !err && data && data.timeline.length > 0 && (
        <div className="claw-diag-shell">
          <nav className="claw-diag-index" aria-label="跳转">
            <div className="claw-diag-index-title">时间 · 点击跳转</div>
            {data.timeline.map((row) => (
              <a
                key={row.id}
                href={`#${row.id}`}
                className={row.isError ? "err" : undefined}
              >
                {formatTimelineTime(row.tsMs)}
              </a>
            ))}
          </nav>
          <main className="claw-diag-timeline">
            {data.timeline.map((row) => (
              <article
                key={row.id}
                id={row.id}
                className={`claw-diag-row${row.isError ? " err" : ""}`}
              >
                <time>
                  <a href={`#${row.id}`}>{formatTimelineTime(row.tsMs)}</a>
                </time>
                <div className="claw-diag-tags">
                  {row.tags.map((t) => (
                    <span key={t} className="claw-diag-tag">
                      {t}
                    </span>
                  ))}
                </div>
                <p className={`claw-diag-msg${row.isError ? " err" : ""}`}>
                  <code>{row.message}</code>
                </p>
              </article>
            ))}
          </main>
          <footer className="claw-diag-foot">
            dsId {data.dsId} · gateway execution + events + task
          </footer>
        </div>
      )}
    </div>
  );
}
