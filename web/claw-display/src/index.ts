import "./claw-display.css";
import { DisplayRouter } from "./DisplayRouter";
import { DocumentPane } from "./DocumentPane";
import { routePayload, stripClawOscFrames } from "./parseOscFrames";
import { StatusBar } from "./StatusBar";

export { DisplayRouter, DocumentPane, StatusBar, routePayload, stripClawOscFrames };
export type { CdpEvent, RouteResult } from "./types";

declare global {
  interface Window {
    ClawDisplay: {
      DisplayRouter: typeof DisplayRouter;
      routePayload: typeof routePayload;
      stripClawOscFrames: typeof stripClawOscFrames;
    };
  }
}

if (typeof window !== "undefined") {
  window.ClawDisplay = {
    DisplayRouter,
    routePayload,
    stripClawOscFrames,
  };
}
