import { HttpAgent, type HttpAgentConfig, type RunAgentInput } from "@ag-ui/client";
import { bridgeRunUrl } from "./claw-config";

function enrichRunInput(input: RunAgentInput, dsId: number): RunAgentInput {
  const base =
    input.forwardedProps && typeof input.forwardedProps === "object"
      ? (input.forwardedProps as Record<string, unknown>)
      : {};
  // L1: threadId → bridge → claw-session-id header only. Do not mirror threadId into
  // forwardedProps.sessionId — that means explicit gateway continuation and 400s when PG
  // has a row but gateway SQLite was reset or the session never solved.
  const { sessionId: _drop, ...rest } = base;
  return {
    ...input,
    forwardedProps: {
      ...rest,
      dsId,
    },
  };
}

/** Injects dsId into every AG-UI run body (bridge requires forwardedProps.dsId). Author: kejiqing */
class ClawHttpAgent extends HttpAgent {
  clawDsId: number;

  constructor(dsId: number, config: HttpAgentConfig) {
    super(config);
    this.clawDsId = dsId;
  }

  run(input: RunAgentInput) {
    return super.run(enrichRunInput(input, this.clawDsId));
  }

  clone(): ClawHttpAgent {
    const cloned = super.clone() as ClawHttpAgent;
    cloned.clawDsId = this.clawDsId;
    return cloned;
  }
}

export function createClawHttpAgent(dsId: number): ClawHttpAgent {
  return new ClawHttpAgent(dsId, { url: bridgeRunUrl() });
}
