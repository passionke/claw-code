/** Preflight plugin registry + pipeline types. Author: kejiqing */

export type PreflightScope = "every_turn" | "session_first_turn";

export interface PreflightImplJson {
  type: "builtin" | "subprocess";
  handler?: string;
  command?: string[];
}

export interface PreflightStepJson {
  pluginId: string;
  scope: PreflightScope;
  impl?: PreflightImplJson;
  config?: Record<string, unknown>;
}

export interface SolvePreflightJson {
  kind?: "none" | string;
  kinds?: string[];
  steps?: PreflightStepJson[];
}

export interface PreflightPluginRecord {
  pluginId: string;
  displayName: string;
  spiVersion: string;
  defaultImpl?: PreflightImplJson;
  configSchema?: Record<string, unknown>;
}

export interface PreflightPluginListResponse {
  plugins: PreflightPluginRecord[];
}

const BUILTIN_SQLBOT = "sqlbot_mcp_start";
const BUILTIN_TURN_LANGUAGE = "turn_language";

/** Normalize legacy `kinds` / `kind` into editable `steps` for Admin UI. */
export function normalizeSolvePreflightSteps(raw?: SolvePreflightJson): PreflightStepJson[] {
  if (!raw) return [];
  if (Array.isArray(raw.steps) && raw.steps.length > 0) {
    return raw.steps.map((s) => ({
      pluginId: s.pluginId,
      scope: s.scope ?? "session_first_turn",
      impl: s.impl,
      config: s.config ?? {},
    }));
  }
  const kinds = Array.isArray(raw.kinds)
    ? raw.kinds.filter((k) => k && k !== "none")
    : raw.kind && raw.kind !== "none"
    ? [raw.kind]
    : [];
  if (kinds.length === 0) return [];
  const steps: PreflightStepJson[] = [
    {
      pluginId: BUILTIN_TURN_LANGUAGE,
      scope: "every_turn",
      impl: { type: "builtin", handler: BUILTIN_TURN_LANGUAGE },
    },
  ];
  for (const k of kinds) {
    if (k === BUILTIN_TURN_LANGUAGE) continue;
    steps.push({
      pluginId: k,
      scope: k === BUILTIN_SQLBOT ? "session_first_turn" : "session_first_turn",
      impl: { type: "builtin", handler: k },
    });
  }
  return steps;
}

export function stepsToSolvePreflightJson(steps: PreflightStepJson[]): SolvePreflightJson {
  const cleaned = steps
    .map((s) => ({
      pluginId: String(s.pluginId || "").trim(),
      scope: s.scope,
      impl: s.impl,
      config: s.config ?? {},
    }))
    .filter((s) => s.pluginId.length > 0);
  if (cleaned.length === 0) {
    return { kind: "none", steps: [] };
  }
  return { steps: cleaned };
}
