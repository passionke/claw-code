"use client";

import { useCallback, useEffect, useState } from "react";
import { fetchConversationIndex, subscribeStore } from "@/lib/claw-conversation-client";
import type { ClawSessionSummary } from "@/lib/claw-conversation-types";
import { projectIdFromDsId } from "@/lib/claw-conversation-types";
import { useClawUi } from "./ClawCopilotProvider";

/** Sidebar session history (PostgreSQL via BFF). Author: kejiqing */
export function ClawConversationList() {
  const { dsId, threadId, newSession, selectSession } = useClawUi();
  const projectId = projectIdFromDsId(dsId);
  const [sessions, setSessions] = useState<ClawSessionSummary[]>([]);
  const [loadErr, setLoadErr] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const data = await fetchConversationIndex(projectId);
      setSessions(data.sessions);
      setLoadErr(null);
    } catch (e) {
      setLoadErr(e instanceof Error ? e.message : String(e));
    }
  }, [projectId]);

  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => subscribeStore(() => void reload()), [reload]);

  return (
    <div className="claw-conv-list" data-testid="claw-conversation-list">
      <div className="claw-conv-list-head">
        <span className="claw-conv-list-title">对话</span>
        <button type="button" className="claw-conv-new" onClick={onNew}>
          新对话
        </button>
      </div>
      {loadErr && <p className="claw-conv-empty">{loadErr}</p>}
      <ul className="claw-conv-items">
        {!loadErr && sessions.length === 0 && (
          <li className="claw-conv-empty">暂无历史，发送消息后会出现在这里</li>
        )}
        {sessions.map((s) => (
          <li key={s.sessionId}>
            <button
              type="button"
              className={`claw-conv-item${s.sessionId === threadId ? " active" : ""}`}
              onClick={() => selectSession(s.sessionId)}
              title={s.sessionId}
            >
              <span className="claw-conv-item-title">{s.title}</span>
              <span className="claw-conv-item-meta">
                {new Date(s.updatedAtMs).toLocaleString(undefined, {
                  month: "short",
                  day: "numeric",
                  hour: "2-digit",
                  minute: "2-digit",
                })}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );

  function onNew() {
    newSession();
  }
}
