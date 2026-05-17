"use client";

import { useClawUi } from "@/components/ClawCopilotProvider";
import { useCallback, useEffect, useRef, useState } from "react";

type TaskRecord = {
  status?: string;
  currentTaskDesc?: string | null;
};

const ACTIVE = new Set(["queued", "running"]);
const TERMINAL = new Set(["succeeded", "failed", "cancelled", "idle"]);
const FAST_MS = 1200;
/** After a run finished, occasional poll picks up the next user message. Author: kejiqing */
const SLOW_MS = 8000;

/** Polls gateway task (JSON taskId = sessionId = threadId; per-turn id is runId). Author: kejiqing */
export function ClawTaskBar() {
  const { threadId, dsId } = useClawUi();
  const [task, setTask] = useState<TaskRecord | null>(null);
  const fastTimer = useRef<ReturnType<typeof setInterval> | null>(null);
  const slowTimer = useRef<ReturnType<typeof setInterval> | null>(null);
  const sawWorkRef = useRef(false);

  const clearFast = useCallback(() => {
    if (fastTimer.current) {
      clearInterval(fastTimer.current);
      fastTimer.current = null;
    }
  }, []);

  const clearSlow = useCallback(() => {
    if (slowTimer.current) {
      clearInterval(slowTimer.current);
      slowTimer.current = null;
    }
  }, []);

  useEffect(() => {
    if (!threadId) return;
    let cancelled = false;
    sawWorkRef.current = false;
    clearFast();
    clearSlow();

    const applyRecord = (data: TaskRecord | null) => {
      if (cancelled) return;
      setTask(data);
    };

    const poll = async () => {
      try {
        const qs = new URLSearchParams({ dsId: String(dsId) });
        const res = await fetch(
          `/api/claw/tasks/${encodeURIComponent(threadId)}?${qs}`,
          { cache: "no-store" },
        );
        if (cancelled) return;
        if (res.status === 404) {
          applyRecord(null);
          clearFast();
          return;
        }
        if (!res.ok) {
          applyRecord(null);
          clearFast();
          return;
        }
        const data = (await res.json()) as TaskRecord;
        const status = data.status ?? "unknown";
        if (ACTIVE.has(status)) {
          sawWorkRef.current = true;
          applyRecord(data);
          if (!fastTimer.current) {
            fastTimer.current = setInterval(() => void poll(), FAST_MS);
          }
          return;
        }
        clearFast();
        if (TERMINAL.has(status)) {
          applyRecord(status === "idle" ? null : data);
          return;
        }
        applyRecord(data);
      } catch {
        if (!cancelled) applyRecord(null);
        clearFast();
      }
    };

    void poll();

    slowTimer.current = setInterval(() => {
      if (!sawWorkRef.current || fastTimer.current) return;
      void poll();
    }, SLOW_MS);

    return () => {
      cancelled = true;
      clearFast();
      clearSlow();
    };
  }, [threadId, dsId, clearFast, clearSlow]);

  if (!task) return null;

  const status = task.status ?? "unknown";
  const desc = task.currentTaskDesc?.trim();
  const show = ACTIVE.has(status) || Boolean(desc);
  if (!show) return null;

  return (
    <div className="claw-task-bar" data-testid="claw-task-bar" role="status" aria-live="polite">
      <span className={`claw-task-status claw-task-status--${status}`}>{status}</span>
      {desc ? <span className="claw-task-desc">{desc}</span> : null}
    </div>
  );
}
