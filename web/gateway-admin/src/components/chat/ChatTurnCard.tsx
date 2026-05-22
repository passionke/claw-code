import { Collapse, Space, Typography } from "antd";
import { useEffect, useRef, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useReportStream } from "../../hooks/useReportStream";
import type { ProgressEvent, SolveTask } from "../../types/chat";
import { claudeTapSessionUrl, isValidHttpUrl } from "../../utils/claudeTap";
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

export default function ChatTurnCard({
  sessionId,
  turnId,
  taskId,
  dsId,
  gatewayBase,
  tapLiveBase,
  tapLiveTemplate,
  initialStatus = "queued",
}: ChatTurnCardProps) {
  const [task, setTask] = useState<SolveTask>({
    status: initialStatus,
    hasReport: false,
    currentTaskDesc: "已提交",
    progressHistory: [],
  });
  const [visibleProgressCount, setVisibleProgressCount] = useState(0);
  const [errorText, setErrorText] = useState("");
  const [fallbackOutput, setFallbackOutput] = useState("");
  const reportOpened = useRef(false);
  const reportLenAtPollRef = useRef(0);
  const reportGrowthStallPolls = useRef(0);
  /** Poll loop must read live report state via refs (avoid stale `report.*` in closure). Author: kejiqing */
  const reportLenRef = useRef(0);
  const report = useReportStream(gatewayBase, sessionId, turnId, dsId);

  useEffect(() => {
    reportLenRef.current = report.text.length;
  }, [report.text]);

  useEffect(() => {
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
        if (t.hasReport) {
          if (!reportOpened.current) {
            reportOpened.current = true;
            reportGrowthStallPolls.current = 0;
            reportLenAtPollRef.current = 0;
            report.open();
          } else if ((t.status || "") === "running") {
            const len = reportLenRef.current;
            // ES can stay "open" while proxy buffers; reopen replays PG by seq.
            if (len > 0 && len === reportLenAtPollRef.current) {
              reportGrowthStallPolls.current += 1;
              if (reportGrowthStallPolls.current >= 3) {
                reportGrowthStallPolls.current = 0;
                report.open();
              }
            } else {
              reportGrowthStallPolls.current = 0;
              reportLenAtPollRef.current = len;
            }
          }
        }
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
        const terminal = ["succeeded", "failed", "cancelled"].includes(t.status || "");
        if (terminal) {
          // Worker SSE ends without `biz.report.done`; reopen for polish / formal `done`.
          if (t.status === "succeeded" && t.hasReport) {
            reportOpened.current = true;
            report.open();
          }
          if (t.error) {
            setErrorText(JSON.stringify(t.error, null, 2));
          } else if (!t.hasReport && t.result?.outputText) {
            const txt = t.result.outputText;
            setFallbackOutput(txt.slice(0, 8000) + (txt.length > 8000 ? "\n…(截断)" : ""));
          }
          break;
        }
        await new Promise((r) => setTimeout(r, 800));
      }
    })();

    return () => {
      cancelled = true;
      report.close();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gatewayBase, taskId, sessionId, turnId, dsId]);

  const history = task.progressHistory || [];
  const reportActive = report.streaming || report.text.length > 0;

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

      {!reportActive && visibleProgressCount > 0 && (
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
        {(reportActive || report.error) && (
          <div className={styles.section}>
            <div className={styles.sectionLabel}>报告</div>
            <article
              className={`${styles.reportProse} ${report.streaming ? styles.reportStreaming : ""} ${
                report.error ? styles.reportErr : ""
              }`}
            >
              {report.text}
            </article>
          </div>
        )}
        {fallbackOutput && !reportActive && (
          <div className={styles.section}>
            <div className={styles.sectionLabel}>回复</div>
            <Typography.Paragraph className={styles.reportProse}>{fallbackOutput}</Typography.Paragraph>
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
