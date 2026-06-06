"use client";

import { CopilotKit } from "@copilotkit/react-core";
import { CopilotSidebar } from "@copilotkit/react-ui";
import { CLAW_AGENT_ID } from "@/lib/claw-config";
import { ClawCopilotWindow } from "./ClawCopilotWindow";
import { ClawRenderMessage } from "./ClawRenderMessage";
import { ClawMainPanel } from "./ClawMainPanel";
import { ClawShell } from "./ClawShell";
import { useClawUi } from "./ClawCopilotProvider";

function ClawEmptyHeader() {
  return null;
}

type Props = {
  bridgeUrl: string;
  gatewayUrl: string;
  tapUrl: string;
  codeServerUrl?: string;
};

export function ClawHome({ bridgeUrl, gatewayUrl, tapUrl, codeServerUrl }: Props) {
  const { dsId, threadId, switching } = useClawUi();

  return (
    <CopilotKit
      key={threadId}
      threadId={threadId}
      runtimeUrl="/api/copilotkit"
      agent={CLAW_AGENT_ID}
      headers={{
        "x-claw-ds-id": String(dsId),
        "x-claw-thread-id": threadId,
      }}
      properties={{
        forwardedProps: { dsId },
      }}
    >
      <CopilotSidebar
        defaultOpen
        clickOutsideToClose={false}
        hitEscapeToClose={false}
        className="claw-copilot-sidebar"
        Window={ClawCopilotWindow}
        Header={ClawEmptyHeader}
        RenderMessage={ClawRenderMessage}
        labels={{
          title: "Claw",
          initial: "描述你的任务，Agent 会通过 gateway 执行。",
          placeholder: switching ? "切换对话中…" : "输入消息…",
        }}
        instructions="You are Claw, a coding agent. Be concise. Use tools when needed. The user's dsId is in forwardedProps."
        suggestions={[
          { title: "Smoke test", message: "Reply with exactly: ok" },
          { title: "Status", message: "Summarize what you can do in one sentence." },
        ]}
      >
        <ClawShell bridgeUrl={bridgeUrl} gatewayUrl={gatewayUrl}>
          <ClawMainPanel tapUrl={tapUrl} codeServerUrl={codeServerUrl} />
        </ClawShell>
      </CopilotSidebar>
    </CopilotKit>
  );
}
