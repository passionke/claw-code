import type { RuleEditorItem, RuleJsonItem } from "../types/project";

export function slugRuleTitle(title: string): string {
  return (
    title
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9._-]+/g, "-")
      .replace(/^-+|-+$/g, "") || "rule"
  );
}

export function stripRuleFrontmatter(content: string): string {
  const m = String(content || "").match(/^---\r?\n[\s\S]*?\r?\n---\r?\n+([\s\S]*)$/);
  return m ? m[1].trim() : String(content || "").trim();
}

export function buildRuleFileContent(
  title: string,
  body: string,
  scope: string
): string {
  const desc = title.trim() || "rule";
  const always = scope === "ALWAYS";
  const header = always
    ? `---\ndescription: ${desc}\nalwaysApply: true\n---\n\n`
    : `---\ndescription: ${desc}\n---\n\n`;
  return header + String(body || "").trim() + "\n";
}

export function parseRuleJsonItem(obj: RuleJsonItem): RuleEditorItem {
  const ruleId = String(obj.ruleId || "").trim();
  const ruleTitle =
    String(obj.ruleTitle || "").trim() ||
    ruleId ||
    (obj.relativePath || "").replace(/^.*\//, "").replace(/\.mdc?$/, "");
  const ruleScope = String(obj.ruleScope || "ALWAYS").trim() || "ALWAYS";
  const ruleContent = stripRuleFrontmatter(obj.content || "");
  return {
    ruleId: ruleId || slugRuleTitle(ruleTitle),
    ruleTitle,
    ruleScope,
    ruleContent,
  };
}

export function rulesJsonFromList(list: RuleEditorItem[]): RuleJsonItem[] {
  return list.map((r) => {
    const title = r.ruleTitle.trim() || r.ruleId;
    const id = slugRuleTitle(r.ruleId || title);
    return {
      ruleId: id,
      ruleTitle: title,
      ruleScope: r.ruleScope || "ALWAYS",
      relativePath: `.cursor/rules/${id}.mdc`,
      content: buildRuleFileContent(title, r.ruleContent, r.ruleScope || "ALWAYS"),
    };
  });
}
