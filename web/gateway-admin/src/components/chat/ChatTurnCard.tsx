import { StopOutlined } from "@ant-design/icons";
import { Button, Collapse, Popconfirm, Space, Tag, Tooltip, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useBizReportStream } from "../../hooks/useBizReportStream";
import { useAgUiStream } from "../../hooks/useAgUiStream";
import type {
  BizAdviceReportResponse,
  ProgressEvent,
  SolveAsyncResponse,
  SolveTask,
  TurnCancelResponse,
  TurnFeedbackValue,
} from "../../types/chat";
import { claudeTapSessionUrl, isValidHttpUrl } from "../../utils/claudeTap";
import { extractSolveReportMessage } from "../../utils/solveReportBody";
import { isAdminOrigin } from "../../utils/clientOrigin";
import ReportMarkdown from "./ReportMarkdown";
import AskUserA2ui from "./AskUserA2ui";
import ProcessStepsA2ui from "./ProcessStepsA2ui";
import ResponsesStreamBody from "./ResponsesStreamBody";
import type { ResponsesStreamBlock } from "../../utils/responsesSseParse";
import TurnFeedbackButtons from "./TurnFeedbackButtons";
import TurnToolsDrawer from "./TurnToolsDrawer";
import TurnTimelineDrawer from "./TurnTimelineDrawer";
import TurnExtraSessionDrawer from "./TurnExtraSessionDrawer";
import { formatDurationMs } from "../../utils/formatDuration";
import {
  ListSessionPlansResponse,
  planPhaseFromPlanStatus,
} from "../../utils/planConfirm";
import { isEffectiveHistoryTurnView, isTerminalTurnStatus } from "../../utils/turnViewMode";
import {
  deriveTurnCardReportView,
  mergeStreamedIntoHistory,
  shouldConnectLiveReportSse,
} from "../../utils/turnCardReportView";
import styles from "./chat.module.css";

export interface ChatTurnCardProps {
  sessionId: string;
  turnId: string;
  taskId: string;
  projId: number;
  gatewayBase: string;
  tapLiveBase: string;
  tapLiveTemplate: string;
  initialStatus?: string;
  /** `history`：终态只读回放；`queued`/`running` 始终走 live（poll + report SSE，多终端各自订阅）。Author: kejiqing */
  viewMode?: "live" | "history";
  hasReport?: boolean;
  /** 列表接口已带正文时跳过二次请求。Author: kejiqing */
  historicalReport?: string;
  /** failed 时列表已带 `output_json.detail`。Author: kejiqing */
  failureDetail?: string;
  turnFeedback?: TurnFeedbackValue;
  feedbackSubmitting?: boolean;
  onTurnFeedback?: (feedback: TurnFeedbackValue) => void;
  clientOrigin?: string | null;
  extraSession?: Record<string, unknown> | null;
  createdAtMs?: number;
  finishedAtMs?: number | null;
  /** Prebound pool at enqueue (history or solve_async). Author: kejiqing */
  initialPoolId?: string | null;
  /** Ingress gateway at enqueue (turn owner). Author: kejiqing */
  initialGatewayId?: string | null;
  initialGatewayBase?: string | null;
  initialWorkerName?: string | null;
  initialWorkerProfile?: string | null;
  initialWorkerExecUser?: string | null;
  /** After Plan confirm, parent appends T2 turn card. Author: kejiqing */
  onPlanConfirmed?: (res: SolveAsyncResponse) => void;
  /** Live POST /v1/responses timeline. When set, do not open biz.report / AG-UI. Author: kejiqing */
  responsesStream?: {
    blocks: ResponsesStreamBlock[];
    live: boolean;
    error?: string;
  };
}

function todoStatusMark(status: string): string {
  const s = (status || "").toLowerCase();
  if (s === "done") return "✓";
  if (s === "in_progress" || s === "running") return "◐";
  if (s === "failed" || s === "skipped") return "✗";
  return "○";
}

function statusLabel(task: SolveTask): string {
  const st = task.status || "unknown";
  if (st === "awaiting_user") return "等待用户回答";
  if (task.planPhase === "awaiting_confirm") return "待确认方案";
  if (task.planPhase === "planning" && st === "running") return "规划中…";
  if (task.planPhase === "confirmed") return "方案已确认";
  if (st === "queued") return "排队中";
  if (st === "running") {
    if (task.hasReport) return "生成报告中…";
    return task.currentTaskDesc?.trim() || "执行中…";
  }
  if (st === "succeeded") return "已完成";
  if (st === "failed") return "失败";
  if (st === "cancelled") return "已取消";
  return st;
}

/** Surface `GET /v1/tasks` error objects (e.g. e2b `{detail,status_code}`). Author: kejiqing */
function formatTaskError(err: unknown): string {
  if (err == null) return "";
  if (typeof err === "string") return err;
  if (typeof err === "object" && "detail" in err) {
    const detail = (err as { detail?: unknown }).detail;
    if (typeof detail === "string" && detail.trim()) {
      const code = (err as { status_code?: unknown }).status_code;
      return code != null ? `${String(code)} ${detail.trim()}` : detail.trim();
    }
  }
  try {
    return JSON.stringify(err, null, 2);
  } catch {
    return String(err);
  }
}

function gatewayHostLabel(base: string): string {
  const t = base.trim();
  if (!t) return "—";
  try {
    return new URL(t).host;
  } catch {
    return t.replace(/^https?:\/\//i, "").replace(/\/.*$/, "") || t;
  }
}

/** 历史回放：按 turn 从 DB 拉 JSON 报告。Author: kejiqing */
async function fetchHistoryReport(
  gatewayBase: string,
  sessionId: string,
  turnId: string,
  projId: number
): Promise<string> {
  const q = new URLSearchParams({
    sessionId,
    turnId,
    proj_id: String(projId),
    stream: "false",
  });
  const res = await proxyHttp<BizAdviceReportResponse>(
    gatewayBase,
    "GET",
    `/v1/biz_advice_report?${q.toString()}`
  );
  const raw = res.reportText?.trim() ?? "";
  return extractSolveReportMessage(raw);
}

export default function ChatTurnCard({
  sessionId,
  turnId,
  taskId,
  projId,
  gatewayBase,
  tapLiveBase,
  tapLiveTemplate,
  initialStatus = "queued",
  viewMode = "live",
  hasReport = false,
  historicalReport: initialHistoricalReport,
  failureDetail: initialFailureDetail,
  turnFeedback,
  feedbackSubmitting,
  onTurnFeedback,
  clientOrigin,
  extraSession,
  createdAtMs,
  finishedAtMs,
  initialPoolId: _initialPoolId,
  initialGatewayId,
  initialGatewayBase,
  initialWorkerName,
  initialWorkerProfile,
  initialWorkerExecUser,
  onPlanConfirmed,
  responsesStream,
}: ChatTurnCardProps) {
  const prefilledReport = extractSolveReportMessage(initialHistoricalReport?.trim() ?? "");
  const prefilledFailure = initialFailureDetail?.trim() ?? "";
  const initialHistoryMode = isEffectiveHistoryTurnView(viewMode, initialStatus);
  const [task, setTask] = useState<SolveTask>({
    status: initialStatus,
    hasReport: initialHistoryMode && (hasReport || Boolean(prefilledReport)),
    currentTaskDesc: initialHistoryMode ? "历史记录" : "已提交",
    progressHistory: [],
  });
  const turnStatus = task.status ?? initialStatus ?? "";
  const responsesLive = responsesStream != null;
  /** In-page responses card keeps the stream; do not flip to biz_report replay. Author: kejiqing */
  const historyMode = responsesLive
    ? false
    : isEffectiveHistoryTurnView(viewMode, turnStatus);
  const effectiveCreatedAtMs = task.createdAtMs ?? createdAtMs;
  const effectiveFinishedAtMs = task.finishedAtMs ?? finishedAtMs;
  const wallMs =
    effectiveCreatedAtMs != null &&
    effectiveFinishedAtMs != null &&
    effectiveFinishedAtMs >= effectiveCreatedAtMs
      ? effectiveFinishedAtMs - effectiveCreatedAtMs
      : null;
  const [visibleProgressCount, setVisibleProgressCount] = useState(0);
  const [errorText, setErrorText] = useState(prefilledFailure);
  const [fallbackOutput, setFallbackOutput] = useState("");
  const [historyReport, setHistoryReport] = useState(prefilledReport);
  const [historyReportLoading, setHistoryReportLoading] = useState(
    historyMode && !prefilledReport
  );
  const [cancelLoading, setCancelLoading] = useState(false);
  const [confirmLoading, setConfirmLoading] = useState(false);
  const [askLoading, setAskLoading] = useState(false);
  const {
    text: streamText,
    live: streamLive,
    open: openReportStream,
    close: closeReportStream,
    waitForSettled,
    reconcileReport,
  } = useBizReportStream(gatewayBase, sessionId, turnId, projId);

  const agUiEnabled = !responsesLive && shouldConnectLiveReportSse(viewMode, turnStatus);
  const { steps: processSteps } = useAgUiStream(
    gatewayBase,
    sessionId,
    turnId,
    projId,
    agUiEnabled
  );

  // Live: connect report SSE on mount; do not wait for poll → running (user sees stream earlier).
  useEffect(() => {
    if (responsesLive) return;
    if (!shouldConnectLiveReportSse(viewMode, turnStatus)) return;
    openReportStream();
    return () => {
      closeReportStream();
    };
  }, [responsesLive, viewMode, turnStatus, gatewayBase, sessionId, turnId, projId, openReportStream, closeReportStream]);

  useEffect(() => {
    if (responsesLive) {
      setTask((prev) => ({
        ...prev,
        status: initialStatus || prev.status,
      }));
      return;
    }
    const prefilled = extractSolveReportMessage(initialHistoricalReport?.trim() ?? "");
    const resetHistoryMode = isEffectiveHistoryTurnView(viewMode, initialStatus);
    setTask({
      status: initialStatus,
      hasReport: resetHistoryMode && (hasReport || Boolean(prefilled)),
      currentTaskDesc: resetHistoryMode ? "历史记录" : "已提交",
      progressHistory: [],
    });
    setErrorText(prefilledFailure);
    setFallbackOutput("");
    setHistoryReport(prefilled);
    setHistoryReportLoading(resetHistoryMode && !prefilled && !prefilledFailure);
  }, [
    sessionId,
    turnId,
    initialStatus,
    viewMode,
    hasReport,
    initialHistoricalReport,
    initialFailureDetail,
    responsesLive,
  ]);

  useEffect(() => {
    if (responsesLive) return;
    if (!historyMode) return;
    if (prefilledFailure) {
      setErrorText(prefilledFailure);
      setHistoryReportLoading(false);
      return;
    }
    const prefilled = extractSolveReportMessage(initialHistoricalReport?.trim() ?? "");
    if (prefilled) {
      setHistoryReport(prefilled);
      setHistoryReportLoading(false);
      return;
    }

    let cancelled = false;
    (async () => {
      try {
        const body = await fetchHistoryReport(gatewayBase, sessionId, turnId, projId);
        if (cancelled) return;
        if (body) {
          setHistoryReport(body);
        } else if (!hasReport) {
          setErrorText("该轮次无已持久化的报告内容");
        }
      } catch (e) {
        if (!cancelled) {
          setErrorText(String((e as Error).message || e));
        }
      } finally {
        if (!cancelled) setHistoryReportLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [
    responsesLive,
    historyMode,
    gatewayBase,
    sessionId,
    turnId,
    projId,
    hasReport,
    initialHistoricalReport,
    prefilledFailure,
  ]);

  // History / terminal: turns list has no plan fields; hydrate from plans API so「确认执行」可见。
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const res = await proxyHttp<ListSessionPlansResponse>(
          gatewayBase,
          "GET",
          `/v1/sessions/${encodeURIComponent(sessionId)}/plans?proj_id=${encodeURIComponent(String(projId))}`
        );
        if (cancelled) return;
        const plan = (res.plans ?? []).find((p) => p.planTurnId === turnId);
        if (!plan) return;
        setTask((prev) => ({
          ...prev,
          planId: plan.planId,
          planTitle: plan.title ?? prev.planTitle,
          planMarkdown: plan.bodyMarkdown ?? prev.planMarkdown,
          planPhase: planPhaseFromPlanStatus(plan.status),
          planTurnId: plan.planTurnId ?? turnId,
          interactionMode: prev.interactionMode ?? "plan",
          currentTaskDesc:
            plan.status === "awaiting_confirm"
              ? "方案待确认"
              : prev.currentTaskDesc,
        }));
      } catch {
        /* non-fatal: confirm button stays hidden if plans unavailable */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [gatewayBase, sessionId, turnId, projId]);

  useEffect(() => {
    if (historyMode) return;
    if (!taskId) return;

    let cancelled = false;

    const pollOnce = async (): Promise<SolveTask | null> => {
      try {
        const t = await proxyHttp<SolveTask>(
          gatewayBase,
          "GET",
          `/v1/tasks/${encodeURIComponent(taskId)}`
        );
        if (cancelled) return null;
        setTask(t);
        return t;
      } catch (e) {
        if (!cancelled && !responsesLive) {
          setErrorText(String((e as Error).message || e));
        }
        return null;
      }
    };

    (async () => {
      while (!cancelled) {
        const t = await pollOnce();
        if (!t) break;
        const terminal = isTerminalTurnStatus(t.status);
        if (terminal) {
          // Responses cards do not wait for biz.report.done. Author: kejiqing
          if (!responsesLive) {
            await waitForSettled(2500);
          }
          if (t.result?.outputText) {
            reconcileReport(t.result.outputText);
            const txt = extractSolveReportMessage(t.result.outputText);
            if (txt) {
              setHistoryReport(txt);
              setHistoryReportLoading(false);
            }
          }
          closeReportStream();
          if (t.error) {
            setErrorText(formatTaskError(t.error));
          } else if (t.status === "succeeded" && t.result?.outputText) {
            const txt = extractSolveReportMessage(t.result.outputText);
            if (txt) {
              setFallbackOutput(txt.slice(0, 8000) + (txt.length > 8000 ? "\n…(截断)" : ""));
            }
          } else if (!t.hasReport && t.result?.outputText) {
            const txt = extractSolveReportMessage(t.result.outputText);
            if (txt) {
              setFallbackOutput(txt.slice(0, 8000) + (txt.length > 8000 ? "\n…(截断)" : ""));
            }
          }
          break;
        }
        const delay = (t.status || "") === "running" ? 300 : 800;
        await new Promise((r) => setTimeout(r, delay));
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [gatewayBase, taskId, historyMode, responsesLive, waitForSettled, reconcileReport, closeReportStream]);

  // Preserve streamed text when poll flips status to terminal before DB fetch completes.
  useEffect(() => {
    const merged = mergeStreamedIntoHistory(historyMode, historyReport, streamText);
    if (merged) {
      setHistoryReport(merged);
      setHistoryReportLoading(false);
    }
  }, [historyMode, historyReport, streamText]);

  const st = task.status || "unknown";
  const shownTurnId = turnId || task.turnId || "";
  const responsesTerminal = responsesLive && isTerminalTurnStatus(st);
  const responsesStreamLive = Boolean(responsesStream?.live) && !responsesTerminal;
  const responsesErrorText =
    responsesStream?.error ||
    (responsesTerminal && errorText ? errorText : "");
  const history = task.progressHistory || [];
  const reportView = deriveTurnCardReportView({
    viewMode,
    status: st,
    historyReport,
    streamText,
    streamLive,
    errorText,
    historyReportLoading,
  });
  const { reportText, reportVisible, reportStreaming, showStreamingPlaceholder, showHistoryLoadingPlaceholder } =
    reportView;

  useEffect(() => {
    setVisibleProgressCount((n) => (history.length > n ? history.length : n));
  }, [history.length]);

  const canCancel = !historyMode && (st === "queued" || st === "running");
  const canFeedback =
    Boolean(onTurnFeedback) &&
    (historyMode || isTerminalTurnStatus(st)) &&
    (reportVisible ||
      Boolean(fallbackOutput) ||
      Boolean(errorText) ||
      historyMode ||
      (responsesLive && (responsesStream?.blocks.length ?? 0) > 0));
  const feedbackEditable = isAdminOrigin(clientOrigin);
  const showFeedback = canFeedback && (feedbackEditable || Boolean(turnFeedback));

  const onCancelTurn = useCallback(async () => {
    setCancelLoading(true);
    try {
      const res = await proxyHttp<TurnCancelResponse>(
        gatewayBase,
        "POST",
        `/v1/sessions/${encodeURIComponent(sessionId)}/turns/${encodeURIComponent(turnId)}/cancel?proj_id=${encodeURIComponent(String(projId))}`
      );
      setTask((prev) => ({
        ...prev,
        status: res.status || "cancelled",
        currentTaskDesc: res.cancelApplied ? "已取消" : prev.currentTaskDesc,
      }));
      closeReportStream();
      if (res.cancelApplied) {
        message.success("已取消该轮次");
      } else {
        message.info("该轮次已结束，无需取消");
      }
    } catch (e) {
      message.error(String((e as Error).message || e));
    } finally {
      setCancelLoading(false);
    }
  }, [gatewayBase, sessionId, turnId, projId, closeReportStream]);

  const dotClass = [
    styles.dot,
    st === "queued" ? styles.pulseQueued : "",
    st === "running"
      ? reportStreaming || task.hasReport
        ? styles.pulseReport
        : styles.pulseRunning
      : "",
    st === "succeeded" ? styles.dotOk : "",
    st === "failed" || st === "cancelled" ? styles.dotErr : "",
  ]
    .filter(Boolean)
    .join(" ");

  const sessionHref = claudeTapSessionUrl(sessionId, tapLiveBase, tapLiveTemplate);
  const sessionLinkValid = isValidHttpUrl(sessionHref);

  const progressItems = history.slice(0, visibleProgressCount).map((ev: ProgressEvent, i: number) => ({
    key: String(i),
    label: (
      <span className={styles.progressCollapseLabel}>
        <span
          className={`${styles.kind} ${
            ev.kind === "report_progress"
              ? styles.kindReport
              : ev.kind === "mcp_tool_started"
                ? styles.kindMcp
                : ""
          }`}
        >
          {ev.kind || "event"}
        </span>
        <span className={styles.progressMsg}>{ev.message || ""}</span>
      </span>
    ),
    children: null,
    showArrow: false,
  }));

  const workerName = (task.workerName ?? initialWorkerName ?? "").trim();
  const workerProfile = (task.workerProfile ?? initialWorkerProfile ?? "").trim();
  const workerExecUser = (task.workerExecUser ?? initialWorkerExecUser ?? "").trim();
  const ingressBase = (task.gatewayBase ?? initialGatewayBase ?? "").trim();
  const ingressId = (task.gatewayId ?? initialGatewayId ?? "").trim();
  const gwLabel = gatewayHostLabel(ingressBase);
  const gwTooltip = ingressBase
    ? ingressId
      ? `${ingressId} · ${ingressBase}`
      : ingressBase
    : "未记录接入 gateway（历史 turn 或旧版本 gateway）";

  const workerTag = workerName ? (
    <Tooltip title="exec 当时的 worker 容器名；池回收后容器可能已销毁，仅作历史记录">
      <Tag color="purple" className={styles.turnRouteTag}>
        worker {workerName}
        {workerProfile || workerExecUser
          ? ` (${[workerProfile, workerExecUser].filter(Boolean).join(" / ")})`
          : ""}
      </Tag>
    </Tooltip>
  ) : (
    <Tooltip title="queued 阶段尚无 worker；running 后由 pool 写入 workerName">
      <Tag className={`${styles.turnRouteTag} ${styles.turnRouteTagMuted}`}>
        worker …{workerProfile || workerExecUser
          ? ` (${[workerProfile, workerExecUser].filter(Boolean).join(" / ")})`
          : ""}
      </Tag>
    </Tooltip>
  );

  return (
    <div className={styles.turnCard}>
      <div className={styles.turnTop}>
        <div className={styles.turnIds}>
          <span>
            session{" "}
            {sessionLinkValid ? (
              <a href={sessionHref} target="_blank" rel="noopener noreferrer" title="claude-tap session traces">
                <code>{sessionId}</code>
              </a>
            ) : (
              <code title="claude-tap traces 地址无效">{sessionId}</code>
            )}
          </span>
          <span>
            turn <code>{shownTurnId}</code>
          </span>
        </div>
        <div className={styles.turnRoute}>
          <Tooltip title={gwTooltip}>
            <Tag color="geekblue" className={styles.turnRouteTag}>
              gateway {gwLabel || "—"}
            </Tag>
          </Tooltip>
          {workerTag}
        </div>
        <div className={styles.turnStatus}>
          <span className={dotClass} />
          <span className={`${styles.statusBadge} ${styles[`badge_${st}`] || ""}`}>{st}</span>
          <span className={styles.statusText}>{statusLabel(task)}</span>
          <Space size={8} style={{ marginLeft: "auto" }}>
            {showFeedback ? (
              <TurnFeedbackButtons
                value={turnFeedback}
                loading={feedbackSubmitting}
                readOnly={!feedbackEditable}
                onSubmit={(fb) => onTurnFeedback?.(fb)}
              />
            ) : null}
            {canCancel ? (
              <Popconfirm
                title="取消该轮次？"
                description="将中止 worker 并将状态标为已取消。"
                okText="取消任务"
                cancelText="返回"
                okButtonProps={{ danger: true, loading: cancelLoading }}
                onConfirm={() => void onCancelTurn()}
              >
                <Button
                  size="small"
                  danger
                  icon={<StopOutlined />}
                  loading={cancelLoading}
                >
                  取消
                </Button>
              </Popconfirm>
            ) : null}
            {wallMs != null ? (
              <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                {formatDurationMs(wallMs)}
              </Typography.Text>
            ) : null}
            <TurnExtraSessionDrawer extraSession={extraSession} />
            <TurnTimelineDrawer
              sessionId={sessionId}
              turnId={shownTurnId}
              projId={projId}
              gatewayBase={gatewayBase}
              taskStatus={st}
            />
            <TurnToolsDrawer
              sessionId={sessionId}
              turnId={shownTurnId}
              projId={projId}
              gatewayBase={gatewayBase}
            />
          </Space>
        </div>
      </div>

      {responsesStream ? (
        <ResponsesStreamBody
          blocks={responsesStream.blocks}
          live={responsesStreamLive}
        />
      ) : processSteps.length > 0 ? (
        <ProcessStepsA2ui steps={processSteps} />
      ) : null}

      {task.status === "awaiting_user" && task.askUserQuestionId && !historyMode ? (
        <AskUserA2ui
          questionId={task.askUserQuestionId}
          a2ui={task.askUserA2ui}
          question={task.askUserQuestion}
          options={task.askUserOptions}
          submitting={askLoading}
          onSubmit={({ answer, selected }) => {
            void (async () => {
              if (!task.askUserQuestionId) return;
              setAskLoading(true);
              try {
                await proxyHttp(
                  gatewayBase,
                  "POST",
                  `/v1/sessions/${encodeURIComponent(sessionId)}/turns/${encodeURIComponent(turnId)}/ask-user-answer`,
                  {
                    projId,
                    questionId: task.askUserQuestionId,
                    answer,
                    selected,
                  }
                );
                setTask((prev) => ({
                  ...prev,
                  status: "running",
                  askUserQuestionId: null,
                  askUserQuestion: null,
                  askUserOptions: null,
                  askUserA2ui: null,
                  currentTaskDesc: "继续执行…",
                }));
                message.success("已提交回答");
              } catch (e) {
                message.error(String((e as Error).message || e));
              } finally {
                setAskLoading(false);
              }
            })();
          }}
        />
      ) : null}

      {(task.planMarkdown || task.planTitle || (task.todos && task.todos.length > 0)) && (
        <div className={styles.planOutline}>
          {task.planTitle ? (
            <div className={styles.planTitle}>{task.planTitle}</div>
          ) : null}
          {task.planMarkdown ? (
            <div className={styles.planMarkdownBody}>
              <ReportMarkdown text={task.planMarkdown} />
            </div>
          ) : null}
          {task.planPhase === "awaiting_confirm" && task.planId ? (
            <div className={styles.planConfirmRow}>
              <Button
                type="primary"
                size="small"
                loading={confirmLoading}
                onClick={() => {
                  void (async () => {
                    if (!task.planId) return;
                    setConfirmLoading(true);
                    try {
                      const res = await proxyHttp<SolveAsyncResponse>(
                        gatewayBase,
                        "POST",
                        `/v1/sessions/${encodeURIComponent(sessionId)}/plans/${encodeURIComponent(task.planId)}/confirm`,
                        { projId }
                      );
                      setTask((prev) => ({
                        ...prev,
                        planPhase: "confirmed",
                        currentTaskDesc: "方案已确认",
                      }));
                      onPlanConfirmed?.(res);
                      message.success("已确认，开始执行");
                    } catch (e) {
                      message.error(String((e as Error).message || e));
                    } finally {
                      setConfirmLoading(false);
                    }
                  })();
                }}
              >
                确认执行
              </Button>
            </div>
          ) : null}
          {task.todos && task.todos.length > 0 ? (
            <ul className={styles.planTodos}>
              {task.todos.map((todo) => (
                <li
                  key={todo.id}
                  className={`${styles.planTodo} ${styles[`planTodo_${(todo.status || "pending").toLowerCase()}`] || ""}`}
                >
                  <span className={styles.planTodoMark}>{todoStatusMark(todo.status)}</span>
                  <span className={styles.planTodoTitle}>{todo.title}</span>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      )}

      {!reportVisible && !historyMode && visibleProgressCount > 0 && (
        <div className={styles.progressFeed}>
          <Collapse
            size="small"
            ghost
            items={[
              {
                key: "log",
                label: `执行进度（${visibleProgressCount}）`,
                children: (
                  <div className={styles.progressList}>
                    {progressItems.map((p) => (
                      <div key={p.key}>{p.label}</div>
                    ))}
                  </div>
                ),
              },
            ]}
          />
        </div>
      )}

      <div className={styles.turnBody}>
        {!responsesLive && showHistoryLoadingPlaceholder && (
          <div className={styles.turnBodyPlaceholder}>加载报告中…</div>
        )}
        {!responsesLive && showStreamingPlaceholder && (
          <div className={styles.turnBodyPlaceholder}>报告流式生成中…</div>
        )}
        {!responsesLive &&
          reportVisible &&
          !(task.planPhase === "awaiting_confirm" && task.planMarkdown) && (
          <div className={styles.section}>
            <div className={styles.sectionLabel}>报告</div>
            <ReportMarkdown text={reportText} streaming={reportStreaming} />
          </div>
        )}
        {!responsesLive &&
          fallbackOutput &&
          !reportVisible &&
          !(task.planPhase === "awaiting_confirm" && task.planMarkdown) && (
          <div className={styles.section}>
            <div className={styles.sectionLabel}>回复</div>
            <ReportMarkdown text={fallbackOutput} />
          </div>
        )}
        {(responsesErrorText || (!responsesLive && errorText)) && (
          <div className={styles.section}>
            <div className={styles.sectionLabel}>错误</div>
            <Typography.Paragraph type="danger" style={{ margin: 0, whiteSpace: "pre-wrap" }}>
              {responsesErrorText || errorText}
            </Typography.Paragraph>
          </div>
        )}
      </div>
    </div>
  );
}
