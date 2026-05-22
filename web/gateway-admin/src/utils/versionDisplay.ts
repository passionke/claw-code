/** Human-readable project / entity version labels. Author: kejiqing */

export const DRAFT_REV = "__draft__";

const COMPACT_REV_RE =
  /^(\d{4})(\d{2})(\d{2})(\d{2})(\d{2})(\d{2})(?:-(\d+))?$/;
const DASHED_REV_RE =
  /^(\d{4})-(\d{2})-(\d{2})_(\d{2})-(\d{2})-(\d{2})(?:-(\d+))?$/;

/** Parse legacy `YYYYMMDDHHmmss` or `YYYY-MM-DD_HH-mm-ss` rev ids. */
export function parseRevToDate(rev: string | undefined | null): Date | null {
  const s = (rev || "").trim();
  if (!s || s === DRAFT_REV) return null;
  let m = s.match(COMPACT_REV_RE);
  if (m) {
    const d = new Date(
      Number(m[1]),
      Number(m[2]) - 1,
      Number(m[3]),
      Number(m[4]),
      Number(m[5]),
      Number(m[6])
    );
    return Number.isNaN(d.getTime()) ? null : d;
  }
  m = s.match(DASHED_REV_RE);
  if (m) {
    const d = new Date(
      Number(m[1]),
      Number(m[2]) - 1,
      Number(m[3]),
      Number(m[4]),
      Number(m[5]),
      Number(m[6])
    );
    return Number.isNaN(d.getTime()) ? null : d;
  }
  return null;
}

export function formatMsZh(ms: number): string {
  if (!ms || ms < 1) return "—";
  return new Date(ms).toLocaleString("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

/** Primary label: `2025/05/21 14:30:22` (prefer createdAtMs). */
export function formatVersionTime(
  rev: string | undefined | null,
  createdAtMs?: number | null
): string {
  if ((rev || "").trim() === DRAFT_REV) return "编辑草稿";
  if (createdAtMs && createdAtMs > 0) return formatMsZh(createdAtMs);
  const d = parseRevToDate(rev);
  return d ? formatMsZh(d.getTime()) : (rev || "—").trim();
}

export function versionOptionLabel(opts: {
  rev: string;
  createdAtMs?: number | null;
  note?: string | null;
  tags?: string[];
}): string {
  const parts = [formatVersionTime(opts.rev, opts.createdAtMs)];
  for (const t of opts.tags || []) {
    if (t) parts.push(t);
  }
  if (opts.note?.trim()) parts.push(opts.note.trim());
  return parts.join(" · ");
}

/** Table title line: time + optional short id for disambiguation. */
export function formatVersionTitle(
  rev: string,
  createdAtMs?: number | null,
  opts?: { isDraft?: boolean }
): { primary: string; secondary?: string } {
  if (opts?.isDraft || rev === DRAFT_REV) {
    return { primary: "编辑草稿", secondary: DRAFT_REV };
  }
  const primary = formatVersionTime(rev, createdAtMs);
  const compact = (rev || "").replace(/[-_:]/g, "");
  const fromMs = createdAtMs
    ? formatMsZh(createdAtMs).replace(/[/\s:]/g, "")
    : "";
  const showId =
    rev &&
    (!fromMs || !compact.includes(fromMs.slice(0, 8))) &&
    rev.length > 12;
  return {
    primary,
    secondary: showId ? rev : undefined,
  };
}
