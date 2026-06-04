import { Button, Input, Spin, Tooltip, message } from "antd";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ChatHistorySidebar from "../components/chat/ChatHistorySidebar";
import ChatTurnCard from "../components/chat/ChatTurnCard";
import ChatToolbar from "../components/chat/ChatToolbar";
import ExtraSessionComposer from "../components/chat/ExtraSessionComposer";
import ConversationTranslateModal from "../components/chat/ConversationTranslateModal";
import styles from "../components/chat/chat.module.css";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import { useChatSession } from "../context/ChatSessionContext";
import { useSessionTurnFeedback } from "../hooks/useSessionTurnFeedback";
import type { ListSessionTurnsResponse, SolveAsyncResponse } from "../types/chat";
import type { ConversationTurnInput } from "../utils/collectConversationForTranslate";
import { buildExtraSession } from "../utils/extraSession";
import {
  emptyFieldsRecord,
  loadExtraSessionKvForDs,
  kvFromExtraSession,
  mergeFieldsWithKv,
  saveExtraSessionKvForDs,
  type ExtraSessionKv,
} from "../utils/extraSessionStorage";
import { CLIENT_ORIGIN_GATEWAY_ADMIN, isAdminOrigin } from "../utils/clientOrigin";
import { extractSolveReportMessage } from "../utils/solveReportBody";
import type { TurnFeedbackValue } from "../types/chat";

interface TurnEntry {
  id: string;
  userText: string;
  taskId: string;
  sessionId: string;
  turnId: string;
  initialStatus?: string;
  viewMode?: "live" | "history";
  hasReport?: boolean;
  historicalReport?: string;
  failureDetail?: string;
  clientOrigin?: string | null;
  feedback?: TurnFeedbackValue;
  extraSession?: Record<string, unknown> | null;
  createdAtMs?: number;
  finishedAtMs?: number | null;
}

interface SysEntry {
  id: string;
  kind: "sys";
  tag?: string;
  text: string;
  variant?: "warn" | "err";
}

type ThreadItem = TurnEntry | SysEntry;

/** 输入框上方快捷问句（点击即发送）。Author: kejiqing */
const QUICK_PROMPTS = [
  "最近生意怎么样",
  "哪个菜卖得好",
  "今天营业额多少",
  "和上周比怎么样",
  "哪些时段客流最高",
  "库存或原料有没有要关注的",
] as const;

function isSys(item: ThreadItem): item is SysEntry {
  return "kind" in item && item.kind === "sys";
}

/** solve_async 对话：按时间线 user → assistant 卡片交错展示。Author: kejiqing */
export default function ChatPage() {
  const { gatewayBase, dsId, projectConfig } = useApp();
  const { tapLiveBase, tapLiveTemplate } = useChatSession();
  const [thread, setThread] = useState<ThreadItem[]>([]);
  const [prompt, setPrompt] = useState("");
  const [sending, setSending] = useState(false);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [sessionClientOrigin, setSessionClientOrigin] = useState<string | null>(null);
  const [historyRefreshKey, setHistoryRefreshKey] = useState(0);
  const [loadingHistory, setLoadingHistory] = useState(false);
  const [translateOpen, setTranslateOpen] = useState(false);
  const [extraKv, setExtraKv] = useState<ExtraSessionKv>({});
  const sessionIdRef = useRef<string | null>(null);
  const logEndRef = useRef<HTMLDivElement>(null);

  const fieldDefs = useMemo(
    () =>
      Array.isArray(projectConfig?.extraSessionFieldsJson)
        ? projectConfig.extraSessionFieldsJson.filter((f) => typeof f === "string" && f.trim())
        : [],
    [projectConfig?.extraSessionFieldsJson]
  );

  const composerDisabled =
    sessionClientOrigin != null && !isAdminOrigin(sessionClientOrigin);

  const {
    feedbackByTurn,
    submittingTurnId,
    submitFeedback,
  } = useSessionTurnFeedback(gatewayBase, dsId, activeSessionId, historyRefreshKey);

  const scrollLog = (smooth = true) => {
    requestAnimationFrame(() =>
      logEndRef.current?.scrollIntoView({ behavior: smooth ? "smooth" : "auto", block: "end" })
    );
  };

  const appendSys = useCallback((b: Omit<SysEntry, "id" | "kind">) => {
    setThread((prev) => [...prev, { ...b, kind: "sys", id: `sys-${Date.now()}-${prev.length}` }]);
    scrollLog();
  }, []);

  const prefillExtraFromStorage = useCallback(() => {
    const stored = loadExtraSessionKvForDs(dsId);
    setExtraKv(mergeFieldsWithKv(fieldDefs, stored));
  }, [dsId, fieldDefs]);

  const onNewSession = () => {
    sessionIdRef.current = null;
    setActiveSessionId(null);
    setSessionClientOrigin(null);
    setThread([]);
    prefillExtraFromStorage();
  };

  // New session (no active session): prefill composer from per-ds localStorage. Author: kejiqing
  useEffect(() => {
    if (sessionClientOrigin != null) return;
    if (activeSessionId != null) return;
    const stored = loadExtraSessionKvForDs(dsId);
    setExtraKv(mergeFieldsWithKv(fieldDefs, stored));
  }, [fieldDefs, activeSessionId, sessionClientOrigin, dsId]);

  const loadSessionHistory = useCallback(
    async (sessionId: string, clientOrigin?: string | null) => {
      if (!gatewayBase) {
        message.error("未选择网关");
        return;
      }
      setLoadingHistory(true);
      setThread([]);
      setSessionClientOrigin(clientOrigin ?? null);
      try {
        const res = await proxyHttp<ListSessionTurnsResponse>(
          gatewayBase,
          "GET",
          `/v1/sessions/${encodeURIComponent(sessionId)}/turns?dsId=${encodeURIComponent(String(dsId))}`
        );
        sessionIdRef.current = sessionId;
        setActiveSessionId(sessionId);
        const turns = res.turns ?? [];
        if (!turns.length) {
          setThread([
            {
              kind: "sys",
              id: `sys-empty-${sessionId}`,
              tag: "历史",
              text: "该会话尚无已记录的轮次。",
              variant: "warn",
            },
          ]);
          setExtraKv(emptyFieldsRecord(fieldDefs));
          return;
        }
        setThread(
          turns.map((t) => ({
            id: t.turnId,
            userText: t.userPrompt?.trim() || "（无用户文案）",
            taskId: sessionId,
            sessionId,
            turnId: t.turnId,
            initialStatus: t.status,
            viewMode: "history" as const,
            hasReport: t.hasReport,
            historicalReport: t.reportBody
              ? extractSolveReportMessage(t.reportBody)
              : undefined,
            failureDetail: t.failureDetail?.trim() || undefined,
            clientOrigin: t.clientOrigin ?? undefined,
            feedback: t.feedback ?? undefined,
            extraSession: t.extraSession ?? undefined,
            createdAtMs: t.createdAtMs,
            finishedAtMs: t.finishedAtMs,
          }))
        );
        const lastTurn = turns[turns.length - 1];
        const lastExtra =
          lastTurn?.extraSession && typeof lastTurn.extraSession === "object"
            ? (lastTurn.extraSession as Record<string, unknown>)
            : undefined;
        setExtraKv(kvFromExtraSession(fieldDefs, lastExtra));
      } catch (e) {
        message.error(String((e as Error).message || e));
      } finally {
        setLoadingHistory(false);
      }
    },
    [gatewayBase, dsId, fieldDefs]
  );

  const runSend = async (userText: string) => {
    if (!gatewayBase) {
      message.error("未选择网关");
      return;
    }
    if (composerDisabled) {
      message.warning("外部会话，仅可查看");
      return;
    }
    saveExtraSessionKvForDs(dsId, extraKv);
    const extra = buildExtraSession(extraKv);
    const payload: Record<string, unknown> = {
      dsId,
      userPrompt: userText,
      extraSession: extra,
    };
    if (sessionIdRef.current) payload.sessionId = sessionIdRef.current;

    let asyncRes: SolveAsyncResponse;
    try {
      asyncRes = await proxyHttp<SolveAsyncResponse>(
        gatewayBase,
        "POST",
        "/v1/solve_async",
        payload
      );
    } catch (e) {
      appendSys({
        tag: "solve_async 失败",
        text: String((e as Error).message || e),
        variant: "err",
      });
      return;
    }

    if (!asyncRes?.taskId) {
      appendSys({ tag: "意外响应", text: "缺少 taskId", variant: "err" });
      return;
    }

    sessionIdRef.current = asyncRes.sessionId;
    setActiveSessionId(asyncRes.sessionId);
    setSessionClientOrigin(CLIENT_ORIGIN_GATEWAY_ADMIN);
    setHistoryRefreshKey((k) => k + 1);
    setThread((prev) => [
      ...prev,
      {
        id: asyncRes.turnId,
        userText,
        taskId: asyncRes.taskId,
        sessionId: asyncRes.sessionId,
        turnId: asyncRes.turnId,
        initialStatus: asyncRes.status || "queued",
        viewMode: "live",
        clientOrigin: CLIENT_ORIGIN_GATEWAY_ADMIN,
        extraSession: extra,
        createdAtMs: Date.now(),
      },
    ]);
    scrollLog();
  };

  const onSend = async () => {
    const text = prompt.trim();
    if (!text) return;
    setPrompt("");
    setSending(true);
    try {
      await runSend(text);
    } finally {
      setSending(false);
    }
  };

  const onQuickPrompt = async (text: string) => {
    if (sending || composerDisabled) return;
    setSending(true);
    try {
      await runSend(text);
    } finally {
      setSending(false);
    }
  };

  const threadTurns: ConversationTurnInput[] = thread
    .filter((item): item is TurnEntry => !isSys(item))
    .map((item) => ({
      turnId: item.turnId,
      sessionId: item.sessionId,
      taskId: item.taskId,
      userText: item.userText,
      viewMode: item.viewMode,
      historicalReport: item.historicalReport,
      failureDetail: item.failureDetail,
    }));

  const canTranslate = Boolean(activeSessionId || threadTurns.length > 0);

  return (
    <div className={styles.chatPage}>
      <ChatHistorySidebar
        gatewayBase={gatewayBase}
        dsId={dsId}
        extraSessionFieldDefs={fieldDefs}
        activeSessionId={activeSessionId}
        refreshKey={historyRefreshKey}
        onSelectSession={(id, origin) => void loadSessionHistory(id, origin)}
        onNewSession={onNewSession}
      />
      <div className={styles.chatPageMain}>
        <div className={styles.chatColumn}>
        <div className={styles.chatToolbarRow}>
          <ChatToolbar
            onNewSession={onNewSession}
            onHealth={(t) => appendSys({ tag: "healthz", text: t })}
            onError={(t) => appendSys({ tag: "error", text: t, variant: "err" })}
            onTranslateConversation={() => setTranslateOpen(true)}
            translateDisabled={!canTranslate || loadingHistory}
          />
        </div>
        <div className={styles.chatMain}>
          <div className={styles.chatLog}>
            {loadingHistory ? (
              <div className={styles.chatLogOverlay} aria-busy="true">
                <Spin tip="加载对话…" />
              </div>
            ) : null}
            <div
              className={styles.chatLogInner}
              key={activeSessionId ?? "new-session"}
            >
            {thread.map((item) => {
            if (isSys(item)) {
              return (
                <div
                  key={item.id}
                  className={`${styles.bubbleSys} ${
                    item.variant === "warn"
                      ? styles.bubbleSysWarn
                      : item.variant === "err"
                        ? styles.bubbleSysErr
                        : ""
                  }`}
                >
                  {item.tag ? (
                    <div
                      className={`${styles.bubbleTag} ${
                        item.variant === "warn"
                          ? styles.bubbleTagWarn
                          : item.variant === "err"
                            ? styles.bubbleTagErr
                            : ""
                      }`}
                    >
                      {item.tag}
                    </div>
                  ) : null}
                  {item.text}
                </div>
              );
            }
            return (
              <div key={item.id} className={styles.turnThread}>
                <div className={styles.bubbleUser}>{item.userText}</div>
                <ChatTurnCard
                  taskId={item.taskId}
                  sessionId={item.sessionId}
                  turnId={item.turnId}
                  dsId={dsId}
                  gatewayBase={gatewayBase}
                  tapLiveBase={tapLiveBase}
                  tapLiveTemplate={tapLiveTemplate}
                  initialStatus={item.initialStatus}
                  viewMode={item.viewMode ?? "live"}
                  hasReport={item.hasReport}
                  historicalReport={item.historicalReport}
                  failureDetail={item.failureDetail}
                  clientOrigin={item.clientOrigin}
                  extraSession={item.extraSession}
                  createdAtMs={item.createdAtMs}
                  finishedAtMs={item.finishedAtMs}
                  turnFeedback={feedbackByTurn[item.turnId] ?? item.feedback}
                  feedbackSubmitting={submittingTurnId === item.turnId}
                  onTurnFeedback={(fb) =>
                    void submitFeedback(item.turnId, fb)
                      .then(() => setHistoryRefreshKey((k) => k + 1))
                      .catch((e) => message.error(String((e as Error).message || e)))
                  }
                />
              </div>
            );
          })}
            <div ref={logEndRef} />
            </div>
        </div>
        <div className={styles.composer}>
          <div className={styles.extraSessionRow}>
            <ExtraSessionComposer
              fields={fieldDefs}
              values={extraKv}
              onChange={setExtraKv}
              disabled={composerDisabled}
            />
          </div>
          {composerDisabled ? (
            <Tooltip title="外部会话，仅可查看">
              <span className={styles.composerDisabledHint}>外部会话，不可追问</span>
            </Tooltip>
          ) : null}
          <div className={styles.quickPrompts}>
            {QUICK_PROMPTS.map((q) => (
              <button
                key={q}
                type="button"
                className={styles.quickPromptBtn}
                disabled={sending || composerDisabled}
                onClick={() => void onQuickPrompt(q)}
              >
                {q}
              </button>
            ))}
          </div>
          <div className={styles.composerRow}>
            <Input.TextArea
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              placeholder={
                composerDisabled
                  ? "外部会话，仅可查看历史"
                  : "输入任务描述（自然语言），Enter 发送；Shift+Enter 换行"
              }
              disabled={composerDisabled}
              autoSize={{ minRows: 2, maxRows: 6 }}
              onKeyDown={(ev) => {
                if (ev.key === "Enter" && !ev.shiftKey) {
                  ev.preventDefault();
                  void onSend();
                }
              }}
            />
            <Button
              type="primary"
              loading={sending}
              disabled={composerDisabled}
              onClick={() => void onSend()}
            >
              发送
            </Button>
          </div>
        </div>
        </div>
        </div>
      </div>
      <ConversationTranslateModal
        open={translateOpen}
        onClose={() => setTranslateOpen(false)}
        gatewayBase={gatewayBase}
        dsId={dsId}
        sessionId={activeSessionId}
        threadTurns={threadTurns}
      />
    </div>
  );
}
