import { DocumentPane } from "./DocumentPane";
import { routePayload } from "./parseOscFrames";
import { StatusBar } from "./StatusBar";
import type { CdpEvent } from "./types";

export interface DisplayRouterOptions {
  documentRoot: HTMLElement;
  statusRoot: HTMLElement;
  /** Map session-relative paths to browser URLs for `![alt](workspace:path)` images. */
  workspaceMediaUrl?: (relativePath: string) => string | null;
  /** Fired on each status phase change (composer enable/disable). */
  onStatus?: (phase: "thinking" | "done" | "failed") => void;
}

/** Routes CDP frames to transcript + status; stray PTY text becomes system notes. Author: kejiqing */
export class DisplayRouter {
  private readonly document: DocumentPane;
  private readonly status: StatusBar;
  private readonly onStatus?: (phase: "thinking" | "done" | "failed") => void;

  constructor(opts: DisplayRouterOptions) {
    this.document = new DocumentPane(opts.documentRoot, opts.workspaceMediaUrl);
    this.status = new StatusBar(opts.statusRoot);
    this.onStatus = opts.onStatus;
  }

  reset(): void {
    this.document.clear();
    this.status.reset();
  }

  routePayload(payload: string): void {
    const { frames } = routePayload(payload);
    this.dispatch(frames);
    // IM shell: ignore non-CDP PTY bytes (rustyline echo, prompts, ANSI). Author: kejiqing
  }

  dispatch(frames: CdpEvent[]): void {
    for (const frame of frames) {
      if (frame.ev === "status") {
        this.status.handle(frame);
        this.onStatus?.(frame.phase);
      } else {
        this.document.handle(frame);
      }
    }
  }
}
