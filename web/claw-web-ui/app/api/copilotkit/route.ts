import {
  CopilotRuntime,
  ExperimentalEmptyAdapter,
  copilotRuntimeNextJSAppRouterEndpoint,
} from "@copilotkit/runtime";
import { NextRequest } from "next/server";
import { CLAW_AGENT_ID, defaultDsId } from "@/lib/claw-config";
import { createClawHttpAgent } from "@/lib/claw-agent";

const serviceAdapter = new ExperimentalEmptyAdapter();

function dsIdFromRequest(req: NextRequest): number {
  const header = req.headers.get("x-claw-ds-id");
  if (header) {
    const n = Number.parseInt(header, 10);
    if (Number.isFinite(n) && n > 0) return n;
  }
  const cookie = req.cookies.get("claw_ds_id")?.value;
  if (cookie) {
    const n = Number.parseInt(cookie, 10);
    if (Number.isFinite(n) && n > 0) return n;
  }
  return defaultDsId();
}

export const POST = async (req: NextRequest) => {
  const dsId = dsIdFromRequest(req);
  const runtime = new CopilotRuntime({
    agents: {
      // HttpAgent vs nested @ag-ui/client version mismatch in CopilotKit 1.52
      [CLAW_AGENT_ID]: createClawHttpAgent(dsId) as never,
    },
  });

  const { handleRequest } = copilotRuntimeNextJSAppRouterEndpoint({
    runtime,
    serviceAdapter,
    endpoint: "/api/copilotkit",
  });

  return handleRequest(req);
};
