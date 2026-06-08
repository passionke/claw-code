/**
 * Helpers for Admin entity editors: read/write via GET/PUT project config (row_for_editing).
 * Author: kejiqing
 */

import type { ProjectConfig, SkillJsonItem, SkillRow } from "../types/project";

export function skillRowsFromConfig(cfg: ProjectConfig): SkillRow[] {
  const arr = Array.isArray(cfg.skillsJson) ? cfg.skillsJson : [];
  return arr
    .map((s) => ({
      skill_name: s.skillName,
      skill_content: s.skillContent ?? "",
      enabled: s.enabled,
    }))
    .sort((a, b) => a.skill_name.localeCompare(b.skill_name));
}

export function mergeSkillIntoJson(
  skillsJson: SkillJsonItem[],
  skillName: string,
  skillContent: string,
  enabled?: boolean
): SkillJsonItem[] {
  const prev = skillsJson.find((s) => s.skillName === skillName);
  const effectiveEnabled = enabled ?? prev?.enabled;
  const others = skillsJson.filter((s) => s.skillName !== skillName);
  const item: SkillJsonItem = { skillName, skillContent };
  if (effectiveEnabled === false) item.enabled = false;
  return [...others, item].sort((a, b) => a.skillName.localeCompare(b.skillName));
}

/** CLAUDE.md body for Admin editor: always from project_config (draft or formal), never disk. */
export function claudeMdFromConfig(cfg: ProjectConfig): string {
  return cfg.claudeMd ?? "";
}

/** Normalize editor text for draft PUT: empty / whitespace-only clears the DB override. */
export function normalizeClaudeMdForSave(text: string): string | null {
  return text.trim() === "" ? null : text;
}
