/** Landlock strict DSL types (shared Admin + API). Author: kejiqing */

export interface LandlockDsl {
  enabled: boolean;
  rw: string[];
  ro: string[];
}

export type LandlockDslSource = "systemDefault" | "projectConfig";

export interface WorkerProfileStrictJson {
  useSystemDefault?: boolean;
  landlock?: LandlockDsl;
}

/** Normalized e2b worker SKU (cpu ceil 1..=16; mem ceil 256 MiB .. 16 GiB). Author: kejiqing */
export interface WorkerResourcesJson {
  cpuCount: number;
  memoryMb: number;
}

/** Fingerprint-matched derived template binding (CoW). Author: kejiqing */
export interface WorkerResourceTemplateJson {
  templateId: string;
  buildId?: string | null;
  fingerprint: string;
}

export interface WorkerProfileJson {
  mode: "strict" | "relaxed";
  /** Optional per-project override; omit to inherit global e2bWorker.poolSize. */
  poolSize?: number | null;
  /** When true, Agent mode registers AskUserQuestion (Plan always has it). Author: kejiqing */
  askUserQuestionInAgent?: boolean;
  /**
   * Optional cpu/memory override. Omit/null → global shared template.
   * Gateway ceil-normalizes on write; fingerprint must match resourceTemplate.
   */
  resources?: WorkerResourcesJson | null;
  /** Set by ensure CLI after Template.build; stripped when resources cleared or fingerprint stale. */
  resourceTemplate?: WorkerResourceTemplateJson | null;
  strict?: WorkerProfileStrictJson;
}

export const WORKER_CPU_MAX = 16;
export const WORKER_MEMORY_QUANTUM_MB = 256;
export const WORKER_MEMORY_MAX_MB = 16 * 1024;

/** Mirror gateway ceil rules for Admin preview (gateway is authoritative). Author: kejiqing */
export function normalizeWorkerResourcesClient(
  cpuRaw: number,
  memoryMbRaw: number
): WorkerResourcesJson | { error: string } {
  if (!Number.isFinite(cpuRaw) || cpuRaw <= 0) {
    return { error: "cpuCount 必须是大于 0 的有限数" };
  }
  const cpuCount = Math.ceil(cpuRaw);
  if (cpuCount < 1 || cpuCount > WORKER_CPU_MAX) {
    return { error: `cpuCount 向上取整后须在 1–${WORKER_CPU_MAX}` };
  }
  if (!Number.isFinite(memoryMbRaw) || memoryMbRaw <= 0) {
    return { error: "memoryMb 必须是大于 0 的有限数" };
  }
  const memoryMb =
    Math.ceil(memoryMbRaw / WORKER_MEMORY_QUANTUM_MB) * WORKER_MEMORY_QUANTUM_MB;
  if (memoryMb < WORKER_MEMORY_QUANTUM_MB || memoryMb > WORKER_MEMORY_MAX_MB) {
    return {
      error: `memoryMb 按 ${WORKER_MEMORY_QUANTUM_MB} 向上取整后须在 ${WORKER_MEMORY_QUANTUM_MB}–${WORKER_MEMORY_MAX_MB}`,
    };
  }
  return { cpuCount, memoryMb };
}

export function workerResourcesFingerprint(r: WorkerResourcesJson): string {
  return `cpu${r.cpuCount}-mem${r.memoryMb}`;
}

/** Light client-side validation before PUT (gateway is authoritative). */
export function validateLandlockDslClient(dsl: LandlockDsl): string | null {
  if (!dsl.enabled) return null;
  if (!dsl.rw.some((p) => p.trim() === "${session_root}")) {
    return "rw 必须包含 ${session_root}";
  }
  for (const [kind, paths] of [["rw", dsl.rw], ["ro", dsl.ro]] as const) {
    for (let i = 0; i < paths.length; i++) {
      const p = paths[i]?.trim() ?? "";
      if (!p) return `landlock.${kind}[${i}] 不能为空`;
      if (p.includes("..")) return `landlock.${kind}[${i}] 不能包含 ..`;
      if (p.startsWith("/claw_sessions")) return `landlock.${kind}[${i}] 禁止 /claw_sessions`;
      if (!p.startsWith("/") && !p.startsWith("${")) {
        return `landlock.${kind}[${i}] 必须是绝对路径或变量`;
      }
    }
  }
  return null;
}
