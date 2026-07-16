import { Alert, Button, Input, Spin, Tooltip, message } from "antd";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
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
import { buildExtraSession } from "../utils/extraSession";
import {
  emptyFieldsRecord,
  loadExtraSessionKvForDs,
  kvFromExtraSession,
  mergeFieldsWithKv,
  saveExtraSessionKvForDs,
  type ExtraSessionKv,
} from "../utils/extraSessionStorage";
import {
  CLIENT_ORIGIN_GATEWAY_ADMIN,
  isExternalOrigin,
} from "../utils/clientOrigin";
import { extractSolveReportMessage } from "../utils/solveReportBody";
import { turnViewModeForStatus } from "../utils/turnViewMode";
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
  poolId?: string | null;
  workerName?: string | null;
  workerProfile?: string | null;
  workerExecUser?: string | null;
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
const CHAT_AUDIT_ONLY = false;
export default function ChatPage() {
  const { gatewayBase, projId, projectConfig } = useApp();
  const { tapLiveBase, tapLiveTemplate } = useChatSession();
  const [searchParams, setSearchParams] = useSearchParams();
  const urlSessionId = (searchParams.get("sessionId") ?? "").trim();
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
  /** Avoid re-fetching the same `?sessionId=` deep link (projId-scoped). Author: kejiqing */
  const openedFromUrlRef = useRef("");
  const logEndRef = useRef<HTMLDivElement>(null);

  const setUrlSessionId = useCallback(
    (id: string | null) => {
      const trimmed = (id ?? "").trim();
      setSearchParams(
        (prev) => {
          const next = new URLSearchParams(prev);
          if (trimmed) next.set("sessionId", trimmed);
          else next.delete("sessionId");
          return next;
        },
        { replace: true }
      );
    },
    [setSearchParams]
  );

  const fieldDefs = useMemo(
    () =>
      Array.isArray(projectConfig?.extraSessionFieldsJson)
        ? projectConfig.extraSessionFieldsJson.filter((f) => typeof f === "string" && f.trim())
        : [],
    [projectConfig?.extraSessionFieldsJson]
  );

  const composerDisabled =
    CHAT_AUDIT_ONLY ||
    (activeSessionId != null && isExternalOrigin(sessionClientOrigin));

  const {
    feedbackByTurn,
    submittingTurnId,
    submitFeedback,
  } = useSessionTurnFeedback(gatewayBase, projId, activeSessionId);

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
    const stored = loadExtraSessionKvForDs(projId);
    setExtraKv(mergeFieldsWithKv(fieldDefs, stored));
  }, [projId, fieldDefs]);

  const onNewSession = () => {
    openedFromUrlRef.current = "";
    sessionIdRef.current = null;
    setActiveSessionId(null);
    setSessionClientOrigin(null);
    setThread([]);
    setUrlSessionId(null);
    prefillExtraFromStorage();
  };

  // New session (no active session): prefill composer from per-ds localStorage. Author: kejiqing
  useEffect(() => {
    if (sessionClientOrigin != null) return;
    if (activeSessionId != null) return;
    const stored = loadExtraSessionKvForDs(projId);
    setExtraKv(mergeFieldsWithKv(fieldDefs, stored));
  }, [fieldDefs, activeSessionId, sessionClientOrigin, projId]);

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
          `/v1/sessions/${encodeURIComponent(sessionId)}/turns?proj_id=${encodeURIComponent(String(projId))}`
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
            viewMode: turnViewModeForStatus(t.status),
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
            poolId: t.poolId ?? undefined,
            workerName: t.workerName ?? undefined,
            workerProfile: t.workerProfile ?? undefined,
            workerExecUser: t.workerExecUser ?? undefined,
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
    [gatewayBase, projId, fieldDefs]
  );

  const selectSession = useCallback(
    (sessionId: string, clientOrigin?: string | null) => {
      openedFromUrlRef.current = `${projId}:${sessionId}`;
      setUrlSessionId(sessionId);
      void loadSessionHistory(sessionId, clientOrigin);
    },
    [projId, setUrlSessionId, loadSessionHistory]
  );

  // Deep link: `/admin/chat?sessionId=` → filter + open history. Author: kejiqing
  useEffect(() => {
    if (!gatewayBase || !urlSessionId) return;
    const key = `${projId}:${urlSessionId}`;
    if (openedFromUrlRef.current === key) return;
    openedFromUrlRef.current = key;
    void loadSessionHistory(urlSessionId);
  }, [gatewayBase, urlSessionId, projId, loadSessionHistory]);

  const runSend = async (userText: string) => {
    if (!gatewayBase) {
      message.error("未选择网关");
      return;
    }
    if (composerDisabled) {
      message.warning("外部会话，仅可查看");
      return;
    }
    saveExtraSessionKvForDs(projId, extraKv);
    const extra = buildExtraSession(extraKv);
    const payload: Record<string, unknown> = {
      projId,
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
    openedFromUrlRef.current = `${projId}:${asyncRes.sessionId}`;
    setUrlSessionId(asyncRes.sessionId);
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
        poolId: asyncRes.poolId ?? undefined,
        workerName: asyncRes.workerName ?? undefined,
        workerProfile: asyncRes.workerProfile ?? undefined,
        workerExecUser: asyncRes.workerExecUser ?? undefined,
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

  const canTranslate = Boolean(activeSessionId);

  return (
    <div className={styles.chatPage}>
      <ChatHistorySidebar
        gatewayBase={gatewayBase}
        projId={projId}
        extraSessionFieldDefs={fieldDefs}
        activeSessionId={activeSessionId}
        sessionIdFilter={urlSessionId}
        refreshKey={historyRefreshKey}
        onSelectSession={selectSession}
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
        {CHAT_AUDIT_ONLY ? (
          <Alert
            type="info"
            showIcon
            message="对话页仅用于审计与历史查看"
            description="交互式编码请使用 OVS（/ovs?projId=）。"
            style={{ margin: "0 12px 8px" }}
          />
        ) : null}
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
                  projId={projId}
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
                  initialPoolId={item.poolId}
                  initialWorkerName={item.workerName}
                  initialWorkerProfile={item.workerProfile}
                  initialWorkerExecUser={item.workerExecUser}
                  turnFeedback={feedbackByTurn[item.turnId] ?? item.feedback}
                  feedbackSubmitting={submittingTurnId === item.turnId}
                  onTurnFeedback={(fb) => {
                    const previousFeedback = item.feedback;
                    setThread((prev) =>
                      prev.map((entry) => {
                        if (isSys(entry) || entry.turnId !== item.turnId) return entry;
                        return { ...entry, feedback: fb };
                      })
                    );
                    void submitFeedback(item.turnId, fb)
                      .then(() => setHistoryRefreshKey((k) => k + 1))
                      .catch((e) => {
                        setThread((prev) =>
                          prev.map((entry) => {
                            if (isSys(entry) || entry.turnId !== item.turnId) return entry;
                            return { ...entry, feedback: previousFeedback };
                          })
                        );
                        message.error(String((e as Error).message || e));
                      });
                  }}
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
            <Tooltip
              title={
                CHAT_AUDIT_ONLY
                  ? "交互式编码请使用 Coding 终端"
                  : "外部会话，仅可查看"
              }
            >
              <span className={styles.composerDisabledHint}>
                {CHAT_AUDIT_ONLY
                  ? "只读审计模式，请打开 Coding 终端"
                  : "外部会话，不可追问"}
              </span>
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
                  ? CHAT_AUDIT_ONLY
                    ? "只读审计：请在 Coding 终端进行交互"
                    : "外部会话，仅可查看历史"
                  : "输入任务描述（自然语言），Enter 发送；Shift+Enter 换行"
              }
              disabled={composerDisabled}
              autoSize={{ minRows: 2, maxRows: 6 }}
              onKeyDown={(ev) => {
                // 中文/日文等输入法合成候选词时，回车用于确认候选词，不应触发发送
                if (ev.nativeEvent.isComposing || ev.keyCode === 229) return;
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
        projId={projId}
        sessionId={activeSessionId}
      />
    </div>
  );
}
