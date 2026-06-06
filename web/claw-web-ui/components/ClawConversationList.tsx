"use client";

import { useCallback, useEffect, useState } from "react";
import {
  archiveSessionApi,
  deleteSessionApi,
  fetchConversationIndex,
  notifyStoreUpdated,
  subscribeStore,
} from "@/lib/claw-conversation-client";
import type { ClawSessionSummary } from "@/lib/claw-conversation-types";
import { projectIdFromDsId } from "@/lib/claw-conversation-types";
import { useClawUi } from "./ClawCopilotProvider";

type Props = {
  layout?: "stack" | "rail";
};

/** Session list in agent dock (select / archive / delete). Author: kejiqing */
export function ClawConversationList({ layout = "stack" }: Props) {
  const { dsId, threadId, newSession, selectSession, switching } = useClawUi();
  const projectId = projectIdFromDsId(dsId);
  const [sessions, setSessions] = useState<ClawSessionSummary[]>([]);
  const [loadErr, setLoadErr] = useState<string | null>(null);
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const activeId = threadId ?? pendingId;

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

  useEffect(() => {
    if (threadId) setPendingId(null);
  }, [threadId]);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const onStore = () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void reload(), 300);
    };
    const unsub = subscribeStore(onStore);
    return () => {
      if (timer) clearTimeout(timer);
      unsub();
    };
  }, [reload]);

  return (
    <div
      className={`claw-conv-list${layout === "rail" ? " claw-conv-list--rail" : ""}`}
      data-testid="claw-conversation-list"
    >
      <div className="claw-conv-list-head">
        <button
          type="button"
          className="claw-conv-new-primary"
          onClick={() => void onNew()}
          disabled={switching || !!busyId}
        >
          新建对话
        </button>
      </div>
      {loadErr && <p className="claw-conv-empty">{loadErr}</p>}
      <ul className="claw-conv-items" role="listbox" aria-label="历史对话">
        {!loadErr && sessions.length === 0 && (
          <li className="claw-conv-empty">暂无历史对话</li>
        )}
        {sessions.map((s) => {
          const isActive = s.sessionId === activeId;
          const rowBusy = busyId === s.sessionId;
          return (
            <li key={s.sessionId} role="presentation" className="claw-conv-row">
              <button
                type="button"
                role="option"
                aria-selected={isActive}
                className={`claw-conv-item${isActive ? " active is-active" : ""}`}
                disabled={switching || !!busyId}
                onClick={() => void onSelect(s.sessionId)}
                title={s.sessionId}
              >
                <span className="claw-conv-item-title">{s.title || "新对话"}</span>
              </button>
              <div className="claw-conv-row-actions" aria-label="对话操作">
                <button
                  type="button"
                  className="claw-conv-action"
                  title="归档（从列表隐藏）"
                  disabled={switching || rowBusy}
                  onClick={(e) => {
                    e.stopPropagation();
                    void onArchive(s.sessionId);
                  }}
                >
                  归档
                </button>
                <button
                  type="button"
                  className="claw-conv-action claw-conv-action--danger"
                  title="删除"
                  disabled={switching || rowBusy}
                  onClick={(e) => {
                    e.stopPropagation();
                    void onDelete(s.sessionId, s.title || "新对话");
                  }}
                >
                  删除
                </button>
              </div>
            </li>
          );
        })}
      </ul>
    </div>
  );

  async function onNew() {
    setPendingId(null);
    await newSession();
  }

  async function onSelect(sessionId: string) {
    if (sessionId === threadId) return;
    setPendingId(sessionId);
    await selectSession(sessionId);
  }

  async function onArchive(sessionId: string) {
    setBusyId(sessionId);
    try {
      await archiveSessionApi(projectId, sessionId);
      notifyStoreUpdated();
      if (sessionId === threadId) {
        await newSession();
      }
      await reload();
    } catch (e) {
      setLoadErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  }

  async function onDelete(sessionId: string, label: string) {
    const ok = window.confirm(`确定删除对话「${label}」？此操作不可恢复。`);
    if (!ok) return;
    setBusyId(sessionId);
    try {
      await deleteSessionApi(projectId, sessionId);
      notifyStoreUpdated();
      if (sessionId === threadId) {
        await newSession();
      }
      await reload();
    } catch (e) {
      setLoadErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  }
}
