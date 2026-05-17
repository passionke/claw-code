import { HttpAgent } from "@ag-ui/client";
import { bridgeRunUrl } from "./claw-config";

/** HttpAgent pointed at ag-ui-claw-bridge /v1/agent/run. Author: kejiqing */
export function createClawHttpAgent(dsId?: number): HttpAgent {
  return new HttpAgent({
    url: bridgeRunUrl(),
    ...(dsId != null ? { forwardedProps: { dsId } } : {}),
  });
}
