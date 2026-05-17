"use client";

import { CopilotSidebar } from "@copilotkit/react-ui";
import { ClawShell } from "./ClawShell";
import { useClawUi } from "./ClawCopilotProvider";

type Props = {
  bridgeUrl: string;
  gatewayUrl: string;
  tapUrl: string;
  codeServerUrl?: string;
};

export function ClawHome({ bridgeUrl, gatewayUrl, tapUrl, codeServerUrl }: Props) {
  const { threadId } = useClawUi();

  return (
    <>
      <ClawShell bridgeUrl={bridgeUrl} gatewayUrl={gatewayUrl} threadId={threadId}>
        <main className="claw-main" data-testid="claw-main">
          <h1>Claw agent workspace</h1>
          <p>
            Use the Copilot sidebar to talk to the Claw agent. Requests go through{" "}
            <code>/api/copilotkit</code> → AG-UI bridge → gateway — not directly to gateway JSON.
          </p>
          <p>
            Live tap:{" "}
            <a href={tapUrl} target="_blank" rel="noreferrer">
              {tapUrl}
            </a>
          </p>
          {codeServerUrl && (
            <>
              <p>
                Workspace (read-only):{" "}
                <a href={codeServerUrl} target="_blank" rel="noreferrer">
                  code-server
                </a>
              </p>
              <iframe
                title="code-server"
                className="claw-code-server-frame"
                data-testid="code-server-frame"
                src={codeServerUrl}
              />
            </>
          )}
        </main>
      </ClawShell>
      <CopilotSidebar
        defaultOpen
        labels={{
          title: "Claw",
          initial: "Ask the Claw agent…",
        }}
        clickOutsideToClose={false}
      />
    </>
  );
}
