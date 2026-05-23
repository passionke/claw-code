import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import styles from "./chat.module.css";

export interface ReportMarkdownProps {
  text: string;
  className?: string;
  streaming?: boolean;
}

/** Renders BOSS report / assistant reply as Markdown. Author: kejiqing */
export default function ReportMarkdown({ text, className, streaming }: ReportMarkdownProps) {
  const proseClass = [
    styles.reportProse,
    streaming ? styles.reportStreaming : "",
    className ?? "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <article className={proseClass}>
      <ReactMarkdown remarkPlugins={[remarkGfm]}>{text}</ReactMarkdown>
    </article>
  );
}
