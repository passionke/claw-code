"use client";

import { CopilotSidebar } from "@copilotkit/react-ui";
import { ClawMainPanel } from "./ClawMainPanel";
import { ClawShell } from "./ClawShell";
import { ClawSidebarHeader } from "./ClawSidebarHeader";
type Props = {
  bridgeUrl: string;
  gatewayUrl: string;
  tapUrl: string;
  codeServerUrl?: string;
};

export function ClawHome({ bridgeUrl, gatewayUrl, tapUrl, codeServerUrl }: Props) {
  return (
    <CopilotSidebar
      defaultOpen
      clickOutsideToClose={false}
      hitEscapeToClose={false}
      className="claw-copilot-sidebar"
      Header={ClawSidebarHeader}
      labels={{
        title: "Claw",
        initial: "Ask the Claw agent — tasks run via gateway worker pool.",
        placeholder: "Message Claw…",
      }}
      instructions="You are Claw, a coding agent. Be concise. Use tools when needed. The user's dsId is in forwardedProps."
      suggestions={[
        { title: "Smoke test", message: "Reply with exactly: ok" },
        { title: "Status", message: "Summarize what you can do in one sentence." },
        { title: "Next step", message: "What should I check in the workspace first?" },
      ]}
    >
      <ClawShell bridgeUrl={bridgeUrl} gatewayUrl={gatewayUrl}>
        <ClawMainPanel tapUrl={tapUrl} codeServerUrl={codeServerUrl} />
      </ClawShell>
    </CopilotSidebar>
  );
}
