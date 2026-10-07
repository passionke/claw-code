/** Preflight plugin registry + lifecycle-event pipeline types. Author: kejiqing */

/** Legacy scope (compat). Prefer `on`. */
export type PreflightScope = "every_turn" | "session_first_turn";

/** Closed lifecycle event set (`steps[].on`). */
export type PreflightLifecycleEvent =
  | "worker.init.start"
  | "worker.init.end"
  | "worker.reuse.start"
  | "worker.reuse.end"
  | "session.start"
  | "session.end"
  | "turn.start"
  | "turn.end";

export interface PreflightImplJson {
  type: "builtin" | "subprocess";
  handler?: string;
  command?: string[];
}

export interface PreflightStepJson {
  pluginId: string;
  /** Preferred lifecycle event. Author: kejiqing */
  on?: PreflightLifecycleEvent;
  /** Legacy; mapped when `on` absent. */
  scope?: PreflightScope;
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

export function eventFromScope(scope: PreflightScope): PreflightLifecycleEvent {
  return scope === "session_first_turn" ? "session.start" : "turn.start";
}

export function scopeFromEvent(on: PreflightLifecycleEvent): PreflightScope | undefined {
  if (on === "turn.start") return "every_turn";
  if (on === "session.start") return "session_first_turn";
  return undefined;
}

export function resolvedEvent(step: PreflightStepJson): PreflightLifecycleEvent {
  if (step.on) return step.on;
  if (step.scope) return eventFromScope(step.scope);
  return "turn.start";
}

/** Normalize legacy `kinds` / `kind` / `scope` into editable `steps` with `on`. */
export function normalizeSolvePreflightSteps(raw?: SolvePreflightJson): PreflightStepJson[] {
  if (!raw) return [];
  if (Array.isArray(raw.steps) && raw.steps.length > 0) {
    return raw.steps.map((s) => {
      const on = resolvedEvent(s);
      return {
        pluginId: s.pluginId,
        on,
        scope: scopeFromEvent(on) ?? s.scope,
        impl: s.impl,
        config: s.config ?? {},
      };
    });
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
      on: "turn.start",
      scope: "every_turn",
      impl: { type: "builtin", handler: BUILTIN_TURN_LANGUAGE },
    },
  ];
  for (const k of kinds) {
    if (k === BUILTIN_TURN_LANGUAGE) continue;
    steps.push({
      pluginId: k,
      on: k === BUILTIN_SQLBOT ? "session.start" : "session.start",
      scope: "session_first_turn",
      impl: { type: "builtin", handler: k },
    });
  }
  return steps;
}

export function stepsToSolvePreflightJson(steps: PreflightStepJson[]): SolvePreflightJson {
  const cleaned = steps
    .map((s) => {
      const on = resolvedEvent(s);
      return {
        pluginId: String(s.pluginId || "").trim(),
        on,
        scope: scopeFromEvent(on),
        impl: s.impl,
        config: s.config ?? {},
      };
    })
    .filter((s) => s.pluginId.length > 0);
  if (cleaned.length === 0) {
    return { kind: "none", steps: [] };
  }
  return { steps: cleaned };
}
