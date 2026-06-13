import { StopOutlined } from "@ant-design/icons";
import { Button, Collapse, Popconfirm, Space, Tag, Tooltip, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import { useBizReportStream } from "../../hooks/useBizReportStream";
import type {
  BizAdviceReportResponse,
  ProgressEvent,
  SolveTask,
  TurnCancelResponse,
  TurnFeedbackValue,
} from "../../types/chat";
import { claudeTapSessionUrl, isValidHttpUrl } from "../../utils/claudeTap";
import { extractSolveReportMessage } from "../../utils/solveReportBody";
import { isAdminOrigin } from "../../utils/clientOrigin";
import { gatewayBaseForPoolId } from "../../utils/gatewayClusterOptions";
import ReportMarkdown from "./ReportMarkdown";
import TurnFeedbackButtons from "./TurnFeedbackButtons";
import TurnToolsDrawer from "./TurnToolsDrawer";
import TurnTimelineDrawer from "./TurnTimelineDrawer";
import TurnExtraSessionDrawer from "./TurnExtraSessionDrawer";
import { formatDurationMs } from "../../utils/formatDuration";
import { isHistoryTurnView, isTerminalTurnStatus } from "../../utils/turnViewMode";
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
  initialWorkerName?: string | null;
  initialWorkerIsolation?: string | null;
  initialWorkerExecUser?: string | null;
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
  initialPoolId,
  initialWorkerName,
  initialWorkerIsolation,
  initialWorkerExecUser,
}: ChatTurnCardProps) {
  const { clusterPools } = useApp();
  const prefilledReport = extractSolveReportMessage(initialHistoricalReport?.trim() ?? "");
  const prefilledFailure = initialFailureDetail?.trim() ?? "";
  const initialHistoryMode = isHistoryTurnView(viewMode, initialStatus);
  const [task, setTask] = useState<SolveTask>({
    status: initialStatus,
    hasReport: initialHistoryMode && (hasReport || Boolean(prefilledReport)),
    currentTaskDesc: initialHistoryMode ? "历史记录" : "已提交",
    progressHistory: [],
  });
  const turnStatus = task.status ?? initialStatus ?? "";
  const historyMode = isHistoryTurnView(viewMode, turnStatus);
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
  const reportStream = useBizReportStream(gatewayBase, sessionId, turnId, projId);

  // Live: connect report SSE on mount; do not wait for poll → running (user sees stream earlier).
  useEffect(() => {
    if (historyMode) return;
    reportStream.open();
    return () => {
      reportStream.close();
    };
  }, [historyMode, gatewayBase, sessionId, turnId, projId, reportStream]);

  useEffect(() => {
    const prefilled = extractSolveReportMessage(initialHistoricalReport?.trim() ?? "");
    const resetHistoryMode = isHistoryTurnView(viewMode, initialStatus);
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
  ]);

  useEffect(() => {
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
    historyMode,
    gatewayBase,
    sessionId,
    turnId,
    projId,
    hasReport,
    initialHistoricalReport,
    prefilledFailure,
  ]);

  useEffect(() => {
    if (historyMode) return;

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
        if (!cancelled) setErrorText(String((e as Error).message || e));
        return null;
      }
    };

    (async () => {
      while (!cancelled) {
        const t = await pollOnce();
        if (!t) break;
        const terminal = isTerminalTurnStatus(t.status);
        if (terminal) {
          // Let pool/gateway send `biz.report.done` (full text) before cutting SSE.
          await reportStream.waitForSettled(2500);
          if (t.result?.outputText) {
            reportStream.reconcileReport(t.result.outputText);
          }
          reportStream.close();
          if (t.error) {
            setErrorText(JSON.stringify(t.error, null, 2));
          } else if (t.status === "succeeded" && t.result?.outputText) {
            const txt = extractSolveReportMessage(t.result.outputText);
            if (!reportStream.text && txt) {
              setFallbackOutput(txt.slice(0, 8000) + (txt.length > 8000 ? "\n…(截断)" : ""));
            }
          } else if (!t.hasReport && t.result?.outputText) {
            const txt = extractSolveReportMessage(t.result.outputText);
            if (!reportStream.text && txt) {
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gatewayBase, taskId, historyMode]);

  const history = task.progressHistory || [];
  const liveReportText = reportStream.text;
  const reportText = historyMode ? historyReport : liveReportText;
  const reportVisible = reportText.length > 0;
  const reportStreaming = historyMode ? false : reportStream.live;

  useEffect(() => {
    setVisibleProgressCount((n) => (history.length > n ? history.length : n));
  }, [history.length]);

  const st = task.status || "unknown";
  const canCancel = !historyMode && (st === "queued" || st === "running");
  const canFeedback =
    Boolean(onTurnFeedback) &&
    (historyMode || isTerminalTurnStatus(st)) &&
    (reportVisible || Boolean(fallbackOutput) || Boolean(errorText) || historyMode);
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
      reportStream.close();
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
  }, [gatewayBase, sessionId, turnId, projId, reportStream]);

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

  const poolId = (task.poolId ?? initialPoolId ?? "").trim();
  const workerName = (task.workerName ?? initialWorkerName ?? "").trim();
  const workerIsolation = (task.workerIsolation ?? initialWorkerIsolation ?? "").trim();
  const workerExecUser = (task.workerExecUser ?? initialWorkerExecUser ?? "").trim();
  const turnGatewayBase = gatewayBaseForPoolId(poolId, clusterPools, gatewayBase);
  const gwLabel = gatewayHostLabel(turnGatewayBase);

  const poolTag = !poolId ? (
    <Tag color="warning" className={styles.turnRouteTag}>
      pool —
    </Tag>
  ) : (
    <Tag color="cyan" className={styles.turnRouteTag}>
      pool {poolId}
    </Tag>
  );

  const workerTag = workerName ? (
    <Tooltip title="exec 当时的 worker 容器名；池回收后容器可能已销毁，仅作历史记录">
      <Tag color="purple" className={styles.turnRouteTag}>
        worker {workerName}
        {workerIsolation || workerExecUser
          ? ` (${[workerIsolation, workerExecUser].filter(Boolean).join(" / ")})`
          : ""}
      </Tag>
    </Tooltip>
  ) : (
    <Tooltip title="queued 阶段尚无 worker；running 后由 pool 写入 workerName">
      <Tag className={`${styles.turnRouteTag} ${styles.turnRouteTagMuted}`}>
        worker …{workerIsolation || workerExecUser
          ? ` (${[workerIsolation, workerExecUser].filter(Boolean).join(" / ")})`
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
              <a href={sessionHref} target="_blank" rel="noopener noreferrer" title="claude-tap Live">
                <code>{sessionId}</code>
              </a>
            ) : (
              <code title="claude-tap Live 地址无效">{sessionId}</code>
            )}
          </span>
          <span>
            turn <code>{turnId}</code>
          </span>
        </div>
        <div className={styles.turnRoute}>
          <Tooltip
            title={
              turnGatewayBase !== gatewayBase && gatewayBase
                ? `${turnGatewayBase}（本机 UI 网关 ${gatewayHostLabel(gatewayBase)}）`
                : turnGatewayBase || "未选择网关"
            }
          >
            <Tag color="geekblue" className={styles.turnRouteTag}>
              gateway {gwLabel}
            </Tag>
          </Tooltip>
          {poolTag}
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
              turnId={turnId}
              projId={projId}
              gatewayBase={gatewayBase}
              taskStatus={st}
            />
            <TurnToolsDrawer
              sessionId={sessionId}
              turnId={turnId}
              projId={projId}
              gatewayBase={gatewayBase}
            />
          </Space>
        </div>
      </div>

      {(task.planTitle || (task.todos && task.todos.length > 0)) && (
        <div className={styles.planOutline}>
          {task.planTitle ? (
            <div className={styles.planTitle}>{task.planTitle}</div>
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
        {historyMode && historyReportLoading && !reportVisible && !errorText && (
          <div className={styles.turnBodyPlaceholder}>加载报告中…</div>
        )}
        {!historyMode && reportStreaming && !reportVisible && !errorText && (
          <div className={styles.turnBodyPlaceholder}>报告流式生成中…</div>
        )}
        {reportVisible && (
          <div className={styles.section}>
            <div className={styles.sectionLabel}>报告</div>
            <ReportMarkdown text={reportText} streaming={reportStreaming} />
          </div>
        )}
        {fallbackOutput && !reportVisible && (
          <div className={styles.section}>
            <div className={styles.sectionLabel}>回复</div>
            <ReportMarkdown text={fallbackOutput} />
          </div>
        )}
        {errorText && (
          <div className={styles.section}>
            <div className={styles.sectionLabel}>错误</div>
            <Typography.Paragraph type="danger" style={{ margin: 0, whiteSpace: "pre-wrap" }}>
              {errorText}
            </Typography.Paragraph>
          </div>
        )}
      </div>
    </div>
  );
}
