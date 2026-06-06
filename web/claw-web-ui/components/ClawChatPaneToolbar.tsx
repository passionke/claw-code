"use client";

import { useChatContext } from "@copilotkit/react-ui";
import { useClawUi } from "./ClawCopilotProvider";
import { ClawConversationSync } from "./ClawConversationSync";
import { ClawSessionIdCopy } from "./ClawSessionIdCopy";
import { ClawTaskBar } from "./ClawTaskBar";
import { useActiveSessionTitle } from "@/lib/use-active-session-title";

/** Chat column header inside agent dock. Author: kejiqing */
export function ClawChatPaneToolbar() {
  const { setOpen } = useChatContext();
  const { dsId, threadId, switching } = useClawUi();
  const title = useActiveSessionTitle();

  return (
    <header className="claw-dock-chat-header">
      <div className="claw-dock-chat-header-top">
        <div className="claw-dock-chat-heading">
          <h2 className="claw-dock-chat-title" title={title}>
            {title}
          </h2>
          <span className="claw-dock-chat-sub">Workspace ds{dsId}</span>
        </div>
        <div className="claw-dock-chat-actions">
          <ClawSessionIdCopy sessionId={threadId} dsId={dsId} variant="toolbar" />
          <button
            type="button"
            className="claw-icon-btn"
            aria-label="关闭 Agent 面板"
            onClick={() => setOpen(false)}
          >
            ×
          </button>
        </div>
      </div>
      {switching ? <div className="claw-dock-switching">正在切换对话…</div> : null}
      <ClawTaskBar />
      <ClawConversationSync />
    </header>
  );
}
