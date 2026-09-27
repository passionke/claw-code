/** One Responses SSE timeline: text + thinking/tool/mcp/shell tags. Author: kejiqing */
import { Tag, Typography } from "antd";
import ReportMarkdown from "./ReportMarkdown";
import type { ResponsesStreamBlock } from "../../utils/responsesSseParse";
import styles from "./chat.module.css";

export interface ResponsesStreamBodyProps {
  blocks: ResponsesStreamBlock[];
  live?: boolean;
}

function kindColor(kind: string, status: string): string {
  if (status === "failed") return "error";
  if (status === "running") return "processing";
  if (kind === "shell") return "cyan";
  if (kind === "mcp") return "purple";
  if (kind === "thinking") return "gold";
  if (kind === "search" || kind === "read") return "blue";
  if (kind === "edit") return "orange";
  return "default";
}

function TagCard({
  kind,
  title,
  body,
  display,
  status,
}: {
  kind: string;
  title: string;
  body: string;
  display: "collapsed" | "expanded";
  status: string;
}) {
  return (
    <details className={styles.streamTag} open={display === "expanded"}>
      <summary className={styles.streamTagHead}>
        <Tag color={kindColor(kind, status)} className={styles.streamKindTag}>
          {kind}
        </Tag>
        <span className={styles.streamTagTitle}>{title || kind}</span>
        <Typography.Text type="secondary" className={styles.streamTagStatus}>
          {status === "running" ? "进行中" : status === "failed" ? "失败" : "完成"}
        </Typography.Text>
      </summary>
      {body ? <pre className={styles.streamTagBody}>{body}</pre> : null}
    </details>
  );
}

/** Interleaved assistant stream (text continues; shell refreshes in place). Author: kejiqing */
export default function ResponsesStreamBody({ blocks, live }: ResponsesStreamBodyProps) {
  if (!blocks.length) {
    return live ? (
      <div className={styles.turnBodyPlaceholder}>一条流正在输出…</div>
    ) : null;
  }
  return (
    <div className={styles.streamTimeline} data-protocol="responses">
      {blocks.map((block, index) => {
        if (block.type === "text") {
          const lastText =
            live && index === blocks.length - 1 && block.type === "text";
          return (
            <div key={block.id} className={styles.streamText}>
              <ReportMarkdown text={block.text} streaming={lastText} />
            </div>
          );
        }
        if (block.type === "ask") {
          return (
            <details
              key={block.id}
              className={styles.streamTag}
              open={block.display === "expanded"}
            >
              <summary className={styles.streamTagHead}>
                <Tag color="magenta" className={styles.streamKindTag}>
                  ask
                </Tag>
                <span className={styles.streamTagTitle}>{block.question || "需要确认"}</span>
              </summary>
              {block.options.length ? (
                <pre className={styles.streamTagBody}>{block.options.join("\n")}</pre>
              ) : null}
            </details>
          );
        }
        return (
          <TagCard
            key={block.id}
            kind={block.kind}
            title={block.title}
            body={block.body}
            display={block.display}
            status={block.status}
          />
        );
      })}
    </div>
  );
}
