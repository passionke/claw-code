/** Per-project e2b warm worker admin view. Author: kejiqing */

export interface ProjectE2bWorkerUrls {
  e2bApiUrl: string;
  trafficProxyBase?: string | null;
  sandboxDomain: string;
  /** Relaxed only */
  ttydPublicHost?: string;
  /** Relaxed only */
  ttydWsUrl?: string;
}

export interface ProjectE2bWorkerInfo {
  slotIndex: number;
  activeLeases?: number;
  sandboxId: string;
  workerId: string;
  templateContract: string;
  running: boolean;
  remainingTtlSecs?: number | null;
  updatedAtMs: number;
  urls: ProjectE2bWorkerUrls;
}

export interface WorkerRotationEventPublic {
  event: string;
  sandboxId?: string | null;
  workerId?: string | null;
  templateId?: string | null;
  reason?: string | null;
  atMs: number;
}

export interface ProjectE2bWorkerStatusResponse {
  projId: number;
  workerProfile: "strict" | "relaxed";
  desiredTemplate: string;
  desiredPoolSize: number;
  workers: ProjectE2bWorkerInfo[];
  rotationLog: WorkerRotationEventPublic[];
}

export interface ProjectE2bWorkerResetResponse {
  projId: number;
  ok: boolean;
  workers: ProjectE2bWorkerInfo[];
  rotationLog: WorkerRotationEventPublic[];
}
