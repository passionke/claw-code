/** A2UI host for claw-process/v1 ProcessStep list. Author: kejiqing */
import { Collapse, Tag, Typography } from "antd";
import type { AgUiProcessStep } from "../../hooks/useAgUiStream";
import styles from "./chat.module.css";

export interface ProcessStepsA2uiProps {
  steps: AgUiProcessStep[];
}

function statusColor(status: string): string {
  if (status === "running") return "processing";
  if (status === "error") return "error";
  if (status === "ok") return "success";
  return "default";
}

function formatDur(ms?: number | null): string {
  if (ms == null || !Number.isFinite(ms)) return "";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

/** Mid-run process disclosure (L1 list + L2 expand). Author: kejiqing */
export default function ProcessStepsA2ui({ steps }: ProcessStepsA2uiProps) {
  if (!steps.length) return null;
  const items = steps.map((s, i) => ({
    key: s.id || String(i),
    label: (
      <span className={styles.processStepLabel}>
        <Tag color={statusColor(s.status)} className={styles.processKindTag}>
          {s.kind || "tool"}
        </Tag>
        <span className={styles.processTitle}>{s.title || s.id}</span>
        {s.status === "running" ? (
          <Typography.Text type="secondary" className={styles.processDur}>
            running…
          </Typography.Text>
        ) : formatDur(s.durationMs) ? (
          <Typography.Text type="secondary" className={styles.processDur}>
            {formatDur(s.durationMs)}
          </Typography.Text>
        ) : null}
      </span>
    ),
    children: s.summary ? (
      <pre className={styles.processSummary}>{s.summary}</pre>
    ) : (
      <Typography.Text type="secondary">无摘要</Typography.Text>
    ),
  }));

  return (
    <div className={styles.processPanel} data-catalog="claw-process/v1">
      <div className={styles.processPanelTitle}>过程</div>
      <Collapse
        size="small"
        ghost
        items={items}
        className={styles.processCollapse}
      />
    </div>
  );
}
