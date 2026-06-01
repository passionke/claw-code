import { CopyOutlined, TranslationOutlined } from "@ant-design/icons";
import { Button, Modal, Progress, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import {
  collectConversationTurns,
  loadSessionTurnsForTranslate,
  type ConversationTurnInput,
} from "../../utils/collectConversationForTranslate";
import {
  formatTranslatedConversation,
  translateConversationTurns,
  type TranslatedTurn,
} from "../../utils/translateToZh";
import ReportMarkdown from "./ReportMarkdown";
import styles from "./chat.module.css";

export interface ConversationTranslateModalProps {
  open: boolean;
  onClose: () => void;
  gatewayBase: string;
  dsId: number;
  sessionId: string | null;
  threadTurns: ConversationTurnInput[];
}

type Phase = "idle" | "collect" | "translate" | "done" | "error";

/** 整通对话译中文（纯前端）。Author: kejiqing */
export default function ConversationTranslateModal({
  open,
  onClose,
  gatewayBase,
  dsId,
  sessionId,
  threadTurns,
}: ConversationTranslateModalProps) {
  const [phase, setPhase] = useState<Phase>("idle");
  const [progressPct, setProgressPct] = useState(0);
  const [statusText, setStatusText] = useState("");
  const [turns, setTurns] = useState<TranslatedTurn[]>([]);
  const [markdown, setMarkdown] = useState("");
  const [errorText, setErrorText] = useState("");

  const reset = useCallback(() => {
    setPhase("idle");
    setProgressPct(0);
    setStatusText("");
    setTurns([]);
    setMarkdown("");
    setErrorText("");
  }, []);

  const run = useCallback(async () => {
    if (!gatewayBase) {
      setErrorText("未选择网关");
      setPhase("error");
      return;
    }

    reset();
    setPhase("collect");
    setStatusText("正在收集对话正文…");

    try {
      let inputs = threadTurns;
      if (!inputs.length && sessionId) {
        inputs = await loadSessionTurnsForTranslate(gatewayBase, dsId, sessionId);
      }
      if (!inputs.length) {
        setErrorText("当前会话没有可翻译的对话轮次");
        setPhase("error");
        return;
      }

      const blocks = await collectConversationTurns(gatewayBase, dsId, inputs, (done, total) => {
        setProgressPct(Math.round((done / total) * 45));
        setStatusText(`收集正文 ${done}/${total}…`);
      });

      setPhase("translate");
      setStatusText("正在翻译为中文…");
      const translated = await translateConversationTurns(gatewayBase, blocks, ({ doneUnits, totalUnits, detail }) => {
        setProgressPct(45 + Math.round((doneUnits / totalUnits) * 55));
        setStatusText(detail ? `翻译中 ${doneUnits}/${totalUnits} · ${detail}` : `翻译中 ${doneUnits}/${totalUnits}…`);
      });

      const md = formatTranslatedConversation(translated);
      setTurns(translated);
      setMarkdown(md);
      setProgressPct(100);
      setStatusText("完成");
      setPhase("done");
    } catch (e) {
      setErrorText(String((e as Error).message || e));
      setPhase("error");
    }
  }, [gatewayBase, dsId, sessionId, threadTurns, reset]);

  useEffect(() => {
    if (open) void run();
    else reset();
  }, [open, run, reset]);

  const onCopy = async () => {
    if (!markdown) return;
    try {
      await navigator.clipboard.writeText(markdown);
      message.success("已复制译文");
    } catch {
      message.error("复制失败");
    }
  };

  const busy = phase === "collect" || phase === "translate";

  return (
    <Modal
      title={
        <span>
          <TranslationOutlined style={{ marginRight: 8 }} />
          整通对话 · 中文译文
        </span>
      }
      open={open}
      onCancel={onClose}
      width={720}
      destroyOnClose
      footer={
        <div className={styles.translateModalFooter}>
          <Button onClick={onClose}>关闭</Button>
          <Button onClick={() => void run()} loading={busy} disabled={busy}>
            重新翻译
          </Button>
          <Button
            type="primary"
            icon={<CopyOutlined />}
            disabled={!markdown || busy}
            onClick={() => void onCopy()}
          >
            复制全文
          </Button>
        </div>
      }
    >
      {busy ? (
        <div className={styles.translateModalProgress}>
          <Progress percent={progressPct} status="active" />
          <Typography.Text type="secondary">{statusText}</Typography.Text>
        </div>
      ) : null}

      {phase === "error" ? (
        <Typography.Paragraph type="danger" style={{ marginBottom: 0 }}>
          {errorText}
        </Typography.Paragraph>
      ) : null}

      {phase === "done" && turns.length > 0 ? (
        <div className={styles.translateModalBody}>
          {turns.map((t) => (
            <section key={t.turnId} className={styles.translateTurnBlock}>
              <div className={styles.translateTurnTitle}>轮次 {t.index}</div>
              <div className={styles.translateSectionLabel}>用户</div>
              <div className={styles.translateUserText}>{t.userTextZh}</div>
              <div className={styles.translateSectionLabel}>助手</div>
              <ReportMarkdown text={t.assistantTextZh} />
            </section>
          ))}
        </div>
      ) : null}
    </Modal>
  );
}
