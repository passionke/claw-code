"use client";

import { useChatContext } from "@copilotkit/react-ui";
import { useClawUi } from "./ClawCopilotProvider";
import { ClawConversationList } from "./ClawConversationList";
import { ClawConversationSync } from "./ClawConversationSync";
import { ClawSessionIdCopy } from "./ClawSessionIdCopy";
import { ClawTaskBar } from "./ClawTaskBar";

export function ClawSidebarHeader() {
  const { setOpen } = useChatContext();
  const { dsId, threadId } = useClawUi();

  return (
    <div className="claw-sidebar-header-wrap">
      <div className="claw-sidebar-header">
        <div className="claw-sidebar-header-text">
          <span className="claw-sidebar-title">Claw Agent</span>
          <span className="claw-sidebar-subtitle">dsId {dsId} · AG-UI → gateway</span>
        </div>
        <button
          type="button"
          className="claw-sidebar-close"
          aria-label="Close sidebar"
          onClick={() => setOpen(false)}
        >
          ×
        </button>
      </div>
      <ClawConversationList />
      <ClawSessionIdCopy sessionId={threadId} dsId={dsId} />
      <ClawTaskBar />
      <ClawConversationSync />
    </div>
  );
}
