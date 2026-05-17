"use client";

import { useCallback, useEffect, useState } from "react";
import { SettingsPanel } from "./SettingsPanel";

type Health = "loading" | "ok" | "err";

function useHealth(url: string, label: string): { label: string; status: Health } {
  const [status, setStatus] = useState<Health>("loading");

  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      try {
        const res = await fetch(`${url}/healthz`, { cache: "no-store" });
        if (!cancelled) setStatus(res.ok ? "ok" : "err");
      } catch {
        if (!cancelled) setStatus("err");
      }
    };
    void run();
    const id = setInterval(run, 15_000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [url]);

  return { label, status };
}

type Props = {
  bridgeUrl: string;
  gatewayUrl: string;
  threadId: string | null;
  children: React.ReactNode;
};

export function ClawShell({
  bridgeUrl,
  gatewayUrl,
  threadId,
  children,
}: Props) {
  const bridge = useHealth(bridgeUrl, "Bridge");
  const gateway = useHealth(gatewayUrl, "Gateway");

  const pill = (name: string, s: Health) => (
    <span
      className={`claw-health-pill claw-health-${s}`}
      data-testid={`health-${name.toLowerCase()}`}
    >
      {name}: {s === "loading" ? "…" : s}
    </span>
  );

  return (
    <div className="claw-shell">
      <header className="claw-header" data-testid="claw-header">
        <span className="claw-brand">Claw Web</span>
        {pill("Bridge", bridge.status)}
        {pill("Gateway", gateway.status)}
        {threadId && (
          <span className="claw-thread" data-testid="thread-id" title={threadId}>
            thread: {threadId.slice(0, 8)}…
          </span>
        )}
        <SettingsPanel />
      </header>
      <div className="claw-body">{children}</div>
    </div>
  );
}
