/** Skill/rule/MCP enabled flag helpers. Author: kejiqing */

export function entityEnabled(enabled?: boolean): boolean {
  return enabled !== false;
}

export function entitySelectLabel(name: string, enabled?: boolean): string {
  return entityEnabled(enabled) ? name : `${name}（已禁用）`;
}
