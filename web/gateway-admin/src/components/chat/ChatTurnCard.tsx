import { Collapse, Space, Typography } from "antd";
import { useEffect, useRef, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useBizReportStream } from "../../hooks/useBizReportStream";
import type { BizAdviceReportResponse, ProgressEvent, SolveTask } from "../../types/chat";
import { claudeTapSessionUrl, isValidHttpUrl } from "../../utils/claudeTap";
import { extractSolveReportMessage } from "../../utils/solveReportBody";
import ReportMarkdown from "./ReportMarkdown";
import TurnToolsDrawer from "./TurnToolsDrawer";
import styles from "./chat.module.css";

export interface ChatTurnCardProps {
  sessionId: string;
  turnId: string;
  taskId: string;
  dsId: number;
  gatewayBase: string;
  tapLiveBase: string;
  tapLiveTemplate: string;
  initialStatus?: string;
  /** `history`：只读回放，按 turn 拉报告，不 poll 最新 task。Author: kejiqing */
  viewMode?: "live" | "history";
  hasReport?: boolean;
  /** 列表接口已带正文时跳过二次请求。Author: kejiqing */
  historicalReport?: string;
  /** failed 时列表已带 `output_json.detail`。Author: kejiqing */
  failureDetail?: string;
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

const TERMINAL = new Set(["succeeded", "failed", "cancelled"]);

/** 历史回放：按 turn 从 DB 拉 JSON 报告。Author: kejiqing */
async function fetchHistoryReport(
  gatewayBase: string,
  sessionId: string,
  turnId: string,
  dsId: number
): Promise<string> {
  const q = new URLSearchParams({
    sessionId,
    turnId,
    dsId: String(dsId),
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
  dsId,
  gatewayBase,
  tapLiveBase,
  tapLiveTemplate,
  initialStatus = "queued",
  viewMode = "live",
  hasReport = false,
  historicalReport: initialHistoricalReport,
  failureDetail: initialFailureDetail,
}: ChatTurnCardProps) {
  const historyMode = viewMode === "history";
  const prefilledReport = extractSolveReportMessage(initialHistoricalReport?.trim() ?? "");
  const prefilledFailure = initialFailureDetail?.trim() ?? "";
  const [task, setTask] = useState<SolveTask>({
    status: initialStatus,
    hasReport: historyMode && (hasReport || Boolean(prefilledReport)),
    currentTaskDesc: historyMode ? "历史记录" : "已提交",
    progressHistory: [],
  });
  const [visibleProgressCount, setVisibleProgressCount] = useState(0);
  const [errorText, setErrorText] = useState(prefilledFailure);
  const [fallbackOutput, setFallbackOutput] = useState("");
  const [historyReport, setHistoryReport] = useState(prefilledReport);
  const [historyReportLoading, setHistoryReportLoading] = useState(
    historyMode && !prefilledReport
  );
  const reportOpened = useRef(false);
  const reportStream = useBizReportStream(gatewayBase, sessionId, turnId, dsId);

  useEffect(() => {
    const prefilled = extractSolveReportMessage(initialHistoricalReport?.trim() ?? "");
    setTask({
      status: initialStatus,
      hasReport: historyMode && (hasReport || Boolean(prefilled)),
      currentTaskDesc: historyMode ? "历史记录" : "已提交",
      progressHistory: [],
    });
    setErrorText(prefilledFailure);
    setFallbackOutput("");
    setHistoryReport(prefilled);
    setHistoryReportLoading(historyMode && !prefilled && !prefilledFailure);
    reportOpened.current = false;
  }, [
    sessionId,
    turnId,
    initialStatus,
    historyMode,
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
        const body = await fetchHistoryReport(gatewayBase, sessionId, turnId, dsId);
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
    dsId,
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
        const terminal = TERMINAL.has(t.status || "");
        if (
          !reportOpened.current &&
          (t.status === "running" || t.status === "queued")
        ) {
          reportOpened.current = true;
          reportStream.open();
        }
        if (terminal) {
          reportStream.close();
          if (t.error) {
            setErrorText(JSON.stringify(t.error, null, 2));
          } else if (t.status === "succeeded" && t.result?.outputText) {
            const txt = t.result.outputText;
            if (!reportStream.text) {
              setFallbackOutput(txt.slice(0, 8000) + (txt.length > 8000 ? "\n…(截断)" : ""));
            }
          } else if (!t.hasReport && t.result?.outputText) {
            const txt = t.result.outputText;
            setFallbackOutput(txt.slice(0, 8000) + (txt.length > 8000 ? "\n…(截断)" : ""));
          }
          break;
        }
        const delay = (t.status || "") === "running" ? 300 : 800;
        await new Promise((r) => setTimeout(r, delay));
      }
    })();

    return () => {
      cancelled = true;
      reportStream.close();
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
  const dotClass = [
    styles.dot,
    st === "queued" ? styles.pulseQueued : "",
    st === "running" ? (task.hasReport ? styles.pulseReport : styles.pulseRunning) : "",
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
        <div className={styles.turnStatus}>
          <span className={dotClass} />
          <span className={`${styles.statusBadge} ${styles[`badge_${st}`] || ""}`}>{st}</span>
          <span className={styles.statusText}>{statusLabel(task)}</span>
          <Space size={8} style={{ marginLeft: "auto" }}>
            <TurnToolsDrawer
              sessionId={sessionId}
              turnId={turnId}
              dsId={dsId}
              gatewayBase={gatewayBase}
            />
          </Space>
        </div>
      </div>

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
