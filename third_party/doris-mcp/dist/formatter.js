/**
 * Format query result as MySQL CLI style.
 * Author: kejiqing
 */
export function formatResult(columns, rows, elapsedMs) {
    if (columns.length === 0) {
        const timeLine = elapsedMs != null ? `\n(execution time: ${formatElapsed(elapsedMs)})` : "";
        return "Empty set." + timeLine;
    }
    const cellStr = (row, col, i) => {
        let v = row[col];
        if (v == null) {
            const keys = Object.keys(row);
            const matchKey = keys.find((k) => k.toLowerCase() === col.toLowerCase());
            if (matchKey !== undefined)
                v = row[matchKey];
            if (v == null && keys.length > 0 && columns.length === 1)
                v = row[keys[0]];
            if (v == null && typeof row[i] !== "undefined")
                v = row[i];
        }
        return v == null ? "NULL" : typeof v === "string" ? v : String(v);
    };
    const colWidths = columns.map((col, i) => {
        let max = col.length;
        for (const row of rows) {
            const s = cellStr(row, col, i);
            if (s.length > max)
                max = s.length;
        }
        return max;
    });
    const sep = colWidths.map((w) => "+" + "-".repeat(w + 2)).join("") + "+";
    const header = "| " +
        columns
            .map((col, i) => pad(String(col), colWidths[i]))
            .join(" | ") +
        " |";
    const lines = [sep, header, sep];
    for (const row of rows) {
        const line = "| " +
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
function formatElapsed(ms) {
    if (ms < 1)
        return `${(ms * 1000).toFixed(0)} µs`;
    if (ms < 1000)
        return `${ms.toFixed(2)} ms`;
    return `${(ms / 1000).toFixed(2)} sec`;
}
function pad(s, width) {
    if (s.length > width)
        return s.slice(0, width);
    return s + " ".repeat(width - s.length);
}
//# sourceMappingURL=formatter.js.map