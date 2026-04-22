/**
 * Format query result as MySQL CLI style.
 * Author: kejiqing
 */

export type Row = Record<string, unknown>;

export function formatResult(
  columns: string[],
  rows: Row[],
  elapsedMs?: number
): string {
  if (columns.length === 0) {
    const timeLine =
      elapsedMs != null ? `\n(execution time: ${formatElapsed(elapsedMs)})` : "";
    return "Empty set." + timeLine;
  }
  const cellStr = (row: Row, col: string, i: number): string => {
    let v = row[col];
    if (v == null) {
      const keys = Object.keys(row);
      const matchKey = keys.find((k) => k.toLowerCase() === col.toLowerCase());
      if (matchKey !== undefined) v = row[matchKey];
      if (v == null && keys.length > 0 && columns.length === 1) v = row[keys[0]];
      if (v == null && typeof (row as Record<number, unknown>)[i] !== "undefined")
        v = (row as Record<number, unknown>)[i];
    }
    return v == null ? "NULL" : typeof v === "string" ? v : String(v);
  };
  const colWidths = columns.map((col, i) => {
    let max = col.length;
    for (const row of rows) {
      const s = cellStr(row, col, i);
      if (s.length > max) max = s.length;
    }
    return max;
  });
  const sep = colWidths.map((w) => "+" + "-".repeat(w + 2)).join("") + "+";
  const header =
    "| " +
    columns
      .map((col, i) => pad(String(col), colWidths[i]))
      .join(" | ") +
    " |";
  const lines: string[] = [sep, header, sep];
  for (const row of rows) {
    const line =
      "| " +
      columns
        .map((col, i) => pad(cellStr(row, col, i), colWidths[i]))
        .join(" | ") +
      " |";
    lines.push(line);
  }
  lines.push(sep);
  let footer = `${rows.length} row${rows.length !== 1 ? "s" : ""} in set`;
  if (elapsedMs != null) {
    footer += ` (execution time: ${formatElapsed(elapsedMs)})`;
  }
  lines.push(footer);
  return lines.join("\n");
}

function formatElapsed(ms: number): string {
  if (ms < 1) return `${(ms * 1000).toFixed(0)} µs`;
  if (ms < 1000) return `${ms.toFixed(2)} ms`;
  return `${(ms / 1000).toFixed(2)} sec`;
}

function pad(s: string, width: number): string {
  if (s.length > width) return s.slice(0, width);
  return s + " ".repeat(width - s.length);
}
