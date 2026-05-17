"use client";

import Link from "next/link";
import { useCallback, useState } from "react";

type Props = {
  sessionId: string | null;
  dsId?: number;
  variant?: "bar" | "inline";
};

/** Gateway `claw-session-id` (= AG-UI threadId). Author: kejiqing */
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

  const rootClass =
    variant === "inline" ? "claw-session-id claw-session-id--inline" : "claw-session-id";

  return (
    <div className={rootClass} data-testid="claw-session-id">
      <span className="claw-session-id-label">claw-session-id</span>
      <code className="claw-session-id-value" title={sessionId}>
        {sessionId}
      </code>
      <button type="button" className="claw-session-id-copy" onClick={() => void copy()}>
        {copied ? "Copied" : "Copy"}
      </button>
      {sessionId && (
        <Link
          className="claw-session-id-diag"
          href={`/session/${encodeURIComponent(sessionId)}${dsId != null ? `?dsId=${dsId}` : ""}`}
        >
          诊断
        </Link>
      )}
    </div>
  );
}
