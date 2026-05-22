import { Button, Input, message } from "antd";
import { useCallback, useRef, useState } from "react";
import ChatTurnCard from "../components/chat/ChatTurnCard";
import ChatToolbar from "../components/chat/ChatToolbar";
import styles from "../components/chat/chat.module.css";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import { useChatSession } from "../context/ChatSessionContext";
import type { SolveAsyncResponse } from "../types/chat";
import { buildExtraSession } from "../utils/extraSession";

interface TurnEntry {
  id: string;
  userText: string;
  taskId: string;
  sessionId: string;
  turnId: string;
  initialStatus?: string;
}

interface SysEntry {
  id: string;
  kind: "sys";
  tag?: string;
  text: string;
  variant?: "warn" | "err";
}

type ThreadItem = TurnEntry | SysEntry;

function isSys(item: ThreadItem): item is SysEntry {
  return "kind" in item && item.kind === "sys";
}

/** solve_async 对话：按时间线 user → assistant 卡片交错展示。Author: kejiqing */
export default function ChatPage() {
  const { gatewayBase, dsId } = useApp();
  const { storeId, orgId, tapLiveBase, tapLiveTemplate } = useChatSession();
  const [thread, setThread] = useState<ThreadItem[]>([]);
  const [prompt, setPrompt] = useState("");
  const [sending, setSending] = useState(false);
  const sessionIdRef = useRef<string | null>(null);
  const logEndRef = useRef<HTMLDivElement>(null);

  const scrollLog = () => {
    requestAnimationFrame(() => logEndRef.current?.scrollIntoView({ behavior: "smooth", block: "end" }));
  };

  const appendSys = useCallback((b: Omit<SysEntry, "id" | "kind">) => {
    setThread((prev) => [...prev, { ...b, kind: "sys", id: `sys-${Date.now()}-${prev.length}` }]);
    scrollLog();
  }, []);

  const onNewSession = () => {
    sessionIdRef.current = null;
    appendSys({
      tag: "session",
      text: "已清空本地 sessionId，下一轮将走新会话。",
      variant: "warn",
    });
  };

  const runSend = async (userText: string) => {
    if (!gatewayBase) {
      message.error("未选择网关");
      return;
    }
    const extra = buildExtraSession({ storeId, orgId });
    const payload: Record<string, unknown> = {
      dsId,
      userPrompt: userText,
      extraSession: extra,
    };
    if (sessionIdRef.current) payload.sessionId = sessionIdRef.current;

    let asyncRes: SolveAsyncResponse;
    try {
      asyncRes = await proxyHttp<SolveAsyncResponse>(
        gatewayBase,
        "POST",
        "/v1/solve_async",
        payload
      );
    } catch (e) {
      appendSys({
        tag: "solve_async 失败",
        text: String((e as Error).message || e),
        variant: "err",
      });
      return;
    }

    if (!asyncRes?.taskId) {
      appendSys({ tag: "意外响应", text: "缺少 taskId", variant: "err" });
      return;
    }

    sessionIdRef.current = asyncRes.sessionId;
    setThread((prev) => [
      ...prev,
      {
        id: asyncRes.turnId,
        userText,
        taskId: asyncRes.taskId,
        sessionId: asyncRes.sessionId,
        turnId: asyncRes.turnId,
        initialStatus: asyncRes.status || "queued",
      },
    ]);
    scrollLog();
  };

  const onSend = async () => {
    const text = prompt.trim();
    if (!text) return;
    setPrompt("");
    setSending(true);
    try {
      await runSend(text);
    } finally {
      setSending(false);
    }
  };

  return (
    <div className={styles.chatPage}>
      <div className={styles.chatToolbarRow}>
        <ChatToolbar
          onNewSession={onNewSession}
          onHealth={(t) => appendSys({ tag: "healthz", text: t })}
          onError={(t) => appendSys({ tag: "error", text: t, variant: "err" })}
        />
      </div>
      <div className={styles.chatMain}>
        <div className={styles.chatLog}>
          {thread.map((item) => {
            if (isSys(item)) {
              return (
                <div
                  key={item.id}
                  className={`${styles.bubbleSys} ${
                    item.variant === "warn"
                      ? styles.bubbleSysWarn
                      : item.variant === "err"
                        ? styles.bubbleSysErr
                        : ""
                  }`}
                >
                  {item.tag ? (
                    <div
                      className={`${styles.bubbleTag} ${
                        item.variant === "warn"
                          ? styles.bubbleTagWarn
                          : item.variant === "err"
                            ? styles.bubbleTagErr
                            : ""
                      }`}
                    >
                      {item.tag}
                    </div>
                  ) : null}
                  {item.text}
                </div>
              );
            }
            return (
              <div key={item.id} className={styles.turnThread}>
                <div className={styles.bubbleUser}>{item.userText}</div>
                <ChatTurnCard
                  taskId={item.taskId}
                  sessionId={item.sessionId}
                  turnId={item.turnId}
                  dsId={dsId}
                  gatewayBase={gatewayBase}
                  tapLiveBase={tapLiveBase}
                  tapLiveTemplate={tapLiveTemplate}
                  initialStatus={item.initialStatus}
                />
              </div>
            );
          })}
          <div ref={logEndRef} />
        </div>
        <div className={styles.composer}>
          <Input.TextArea
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            placeholder="输入任务描述（自然语言），Enter 发送；Shift+Enter 换行"
            autoSize={{ minRows: 2, maxRows: 6 }}
            onKeyDown={(ev) => {
              if (ev.key === "Enter" && !ev.shiftKey) {
                ev.preventDefault();
                void onSend();
              }
            }}
          />
          <Button type="primary" loading={sending} onClick={() => void onSend()}>
            发送
          </Button>
        </div>
      </div>
    </div>
  );
}
