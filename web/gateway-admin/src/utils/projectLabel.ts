/** Display label for project list / selector. Author: kejiqing */

import type { ProjectListItem } from "../types/project";

export function formatProjectLabel(p: ProjectListItem): string {
  const ready = p.environmentPrepared ? "就绪" : "未就绪";
  if (p.projectCode?.trim()) {
    return `#${p.projId} · ${p.projectCode.trim()} · ${ready}`;
  }
  return `#${p.projId} · ${ready}`;
}
