/** Turn-card usage footer (agent-style meta under the reply). Author: kejiqing */

import { Tooltip } from "antd";
import type { TurnUsageAggregate } from "../../types/chat";
import {
  formatUsageByModelTooltip,
  usageMetricParts,
} from "../../utils/tokenUsage";
import styles from "./chat.module.css";

export interface TurnUsageFooterProps {
  usage?: TurnUsageAggregate | null;
}

/** Bottom strip: labeled in / out / cache / total. Author: kejiqing */
export default function TurnUsageFooter({ usage }: TurnUsageFooterProps) {
  const parts = usageMetricParts(usage?.usage ?? null);
  const byModelTip = formatUsageByModelTooltip(usage?.usageByModel);
  const tip =
    byModelTip ??
    (parts
      ? "本轮 LLM 用量（observe tap）。prompt 含 cache；cache 为 prompt_tokens_details.cached_tokens。"
      : "尚无 tap 记账");

  return (
    <Tooltip title={<span style={{ whiteSpace: "pre-wrap" }}>{tip}</span>}>
      <footer className={styles.turnUsageFooter} aria-label="token usage">
        <span className={styles.turnUsageBrand}>tokens</span>
        {parts ? (
          <dl className={styles.turnUsageMetrics}>
            {parts.map((p) => (
              <div
                key={p.key}
                className={`${styles.turnUsageMetric} ${
                  p.key === "total" ? styles.turnUsageMetricTotal : ""
                } ${p.key === "cache" && p.value === 0 ? styles.turnUsageMetricMuted : ""}`}
              >
                <dt>{p.label}</dt>
                <dd>{p.display}</dd>
              </div>
            ))}
          </dl>
        ) : (
          <span className={styles.turnUsageEmpty}>—</span>
        )}
      </footer>
    </Tooltip>
  );
}
