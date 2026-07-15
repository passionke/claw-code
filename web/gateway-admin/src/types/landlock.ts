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

export interface WorkerProfileJson {
  mode: "strict" | "relaxed";
  /** Optional per-project override; omit to inherit global e2bWorker.poolSize. */
  poolSize?: number | null;
  strict?: WorkerProfileStrictJson;
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
