"use client";

import Link from "next/link";
import { useCallback, useState } from "react";

type Props = {
  sessionId: string | null;
  dsId?: number;
  variant?: "bar" | "inline" | "toolbar";
};

/** Gateway session id (= AG-UI threadId). Author: kejiqing */
export function ClawSessionIdCopy({ sessionId, dsId, variant = "bar" }: Props) {
  const [copied, setCopied] = useState(false);

  const copy = useCallback(async () => {
    if (!sessionId) return;
    try {
      await navigator.clipboard.writeText(sessionId);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      /* ignore */
    }
  }, [sessionId]);

  if (!sessionId) return null;

  if (variant === "toolbar") {
    return (
      <div className="claw-toolbar-session" data-testid="claw-session-id">
        <button
          type="button"
          className="claw-icon-btn"
          title={sessionId}
          aria-label="复制 session id"
          onClick={() => void copy()}
        >
          {copied ? "✓" : "⎘"}
        </button>
        <Link
          className="claw-icon-btn claw-icon-btn--link"
          href={`/session/${encodeURIComponent(sessionId)}${dsId != null ? `?dsId=${dsId}` : ""}`}
          title="会话诊断"
          aria-label="会话诊断"
        >
          ◷
        </Link>
      </div>
    );
  }

  const rootClass =
    variant === "inline" ? "claw-session-id claw-session-id--inline" : "claw-session-id";

  return (
    <div className={rootClass} data-testid="claw-session-id">
      <span className="claw-session-id-label">session</span>
      <code className="claw-session-id-value" title={sessionId}>
        {sessionId.slice(0, 8)}…
      </code>
      <button type="button" className="claw-session-id-copy" onClick={() => void copy()}>
        {copied ? "已复制" : "复制"}
      </button>
      <Link
        className="claw-session-id-diag"
        href={`/session/${encodeURIComponent(sessionId)}${dsId != null ? `?dsId=${dsId}` : ""}`}
      >
        诊断
      </Link>
    </div>
  );
}
