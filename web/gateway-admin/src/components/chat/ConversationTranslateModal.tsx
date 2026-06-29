import { CopyOutlined, TranslationOutlined } from "@ant-design/icons";
import { Alert, Button, Modal, Spin, Typography, message } from "antd";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  loadConversationTranslateSnapshot,
  triggerConversationTranslate,
  type ConversationTranslateSnapshot,
} from "../../utils/translateToZh";
import ReportMarkdown from "./ReportMarkdown";
import styles from "./chat.module.css";

const POLL_INTERVAL_MS = 1500;

export interface ConversationTranslateModalProps {
  open: boolean;
  onClose: () => void;
  gatewayBase: string;
  projId: number;
  sessionId: string | null;
}

/** 整通对话译中文：前端只触发后端翻译并轮询快照结果。Author: kejiqing */
export default function ConversationTranslateModal({
  open,
  onClose,
  gatewayBase,
  projId,
  sessionId,
}: ConversationTranslateModalProps) {
  const [snapshot, setSnapshot] = useState<ConversationTranslateSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [triggering, setTriggering] = useState(false);
  const [errorText, setErrorText] = useState("");

  const activeRef = useRef(false);
  const pollRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearPoll = useCallback(() => {
    if (pollRef.current) {
      clearTimeout(pollRef.current);
      pollRef.current = null;
    }
  }, []);

  const fetchSnapshot = useCallback(async (): Promise<ConversationTranslateSnapshot | null> => {
    if (!gatewayBase || !sessionId) return null;
    const snap = await loadConversationTranslateSnapshot(gatewayBase, sessionId, projId);
    if (activeRef.current) setSnapshot(snap);
    return snap;
  }, [gatewayBase, sessionId, projId]);

  const schedulePoll = useCallback(() => {
    clearPoll();
    pollRef.current = setTimeout(() => {
      void (async () => {
        try {
          const snap = await fetchSnapshot();
          if (activeRef.current && snap?.status === "translating") schedulePoll();
        } catch (e) {
          if (activeRef.current) setErrorText(String((e as Error).message || e));
        }
      })();
    }, POLL_INTERVAL_MS);
  }, [clearPoll, fetchSnapshot]);

  useEffect(() => {
    if (!open) return;
    activeRef.current = true;
    setErrorText("");
    setSnapshot(null);
    setLoading(true);
    void (async () => {
      try {
        const snap = await fetchSnapshot();
        if (activeRef.current && snap?.status === "translating") schedulePoll();
      } catch (e) {
        if (activeRef.current) setErrorText(String((e as Error).message || e));
      } finally {
        if (activeRef.current) setLoading(false);
      }
    })();
    return () => {
      activeRef.current = false;
      clearPoll();
    };
  }, [open, fetchSnapshot, schedulePoll, clearPoll]);

  const onTrigger = useCallback(async () => {
    if (!gatewayBase || !sessionId) {
      setErrorText("未选择网关或会话");
      return;
    }
    setTriggering(true);
    setErrorText("");
    try {
      await triggerConversationTranslate(gatewayBase, sessionId, projId);
      setSnapshot((prev) => (prev ? { ...prev, status: "translating", error: undefined } : prev));
      await fetchSnapshot();
      schedulePoll();
    } catch (e) {
      const msg = String((e as Error).message || e);
      if (/in progress/i.test(msg)) {
        await fetchSnapshot();
        schedulePoll();
      } else {
        setErrorText(msg);
      }
    } finally {
      setTriggering(false);
    }
  }, [gatewayBase, sessionId, projId, fetchSnapshot, schedulePoll]);

  const onCopy = async () => {
    if (!snapshot?.markdown) return;
    try {
      await navigator.clipboard.writeText(snapshot.markdown);
      message.success("已复制译文");
    } catch {
      message.error("复制失败");
    }
  };

  const status = snapshot?.status;
  const translating = status === "translating";
  const ready = status === "ready";
  const busy = loading || triggering || translating;
  const hasResult = ready && (snapshot?.turns.length ?? 0) > 0;
  const triggerLabel = snapshot ? "重新翻译" : "翻译";
  const snapshotTimeLabel =
    snapshot?.updatedAtMs != null
      ? new Date(snapshot.updatedAtMs).toLocaleString("zh-CN", { hour12: false })
      : null;

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
          <Button onClick={() => void onTrigger()} loading={triggering} disabled={busy}>
            {triggerLabel}
          </Button>
          <Button
            type="primary"
            icon={<CopyOutlined />}
            disabled={!snapshot?.markdown || busy}
            onClick={() => void onCopy()}
          >
            复制全文
          </Button>
        </div>
      }
    >
      {loading ? (
        <div className={styles.translateModalProgress}>
          <Spin />
          <Typography.Text type="secondary">加载译文…</Typography.Text>
        </div>
      ) : null}

      {translating ? (
        <div className={styles.translateModalProgress}>
          <Spin />
          <Typography.Text type="secondary">
            后端翻译中…可关闭本窗口，稍后再回来查看结果。
          </Typography.Text>
        </div>
      ) : null}

      {errorText ? (
        <Typography.Paragraph type="danger" style={{ marginBottom: 0 }}>
          {errorText}
        </Typography.Paragraph>
      ) : null}

      {status === "error" && snapshot?.error ? (
        <Alert
          type="error"
          showIcon
          style={{ marginBottom: 12 }}
          message="翻译失败"
          description={snapshot.error}
        />
      ) : null}

      {!loading && !translating && !errorText && !snapshot ? (
        <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
          尚无译文。点击「翻译」由网关大模型翻译整通会话（仅翻译已完成的轮次）。
        </Typography.Paragraph>
      ) : null}

      {ready && snapshot?.stale ? (
        <Alert
          type="warning"
          showIcon
          style={{ marginBottom: 12 }}
          message="已有新完成的轮次，当前译文可能不完整"
          description="可点击「重新翻译」生成最新中文版并覆盖快照。"
        />
      ) : null}

      {ready && snapshotTimeLabel ? (
        <Typography.Text type="secondary" style={{ display: "block", marginBottom: 12 }}>
          快照时间：{snapshotTimeLabel}
          {snapshot?.stale ? "（可能不完整）" : ""}
        </Typography.Text>
      ) : null}

      {hasResult ? (
        <div className={styles.translateModalBody}>
          {snapshot!.turns.map((t) => (
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
