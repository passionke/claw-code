/** Auto-scale wall-clock duration: ms → s → min → h → day. Author: kejiqing */

function formatSeconds(s: number): string {
  if (Math.abs(s - Math.round(s)) < 0.05) {
    return `${Math.round(s)}s`;
  }
  return `${s.toFixed(1)}s`;
}

/** Format a non-negative duration in milliseconds with the smallest readable unit. */
export function formatDurationMs(ms: number): string {
  if (!Number.isFinite(ms)) return "—";
  const n = Math.max(0, Math.round(ms));
  if (n < 1000) return `${n}ms`;

  const s = n / 1000;
  if (s < 60) return formatSeconds(s);

  const totalMin = Math.floor(s / 60);
  const sec = Math.round(s - totalMin * 60);
  if (totalMin < 60) {
    if (sec === 0) return `${totalMin}m`;
    if (sec === 60) return `${totalMin + 1}m`;
    return `${totalMin}m ${sec}s`;
  }

  const hours = Math.floor(totalMin / 60);
  const min = totalMin - hours * 60;
  if (hours < 24) {
    if (min === 0) return `${hours}h`;
    return `${hours}h ${min}m`;
  }

  const days = Math.floor(hours / 24);
  const h = hours - days * 24;
  if (h === 0) return `${days}d`;
  return `${days}d ${h}h`;
}

/** Relative offset from a phase origin, e.g. "+3ms" or "+1m 21s". */
export function formatOffsetMs(deltaMs: number): string {
  if (!Number.isFinite(deltaMs)) return "—";
  const sign = deltaMs < 0 ? "-" : "+";
  return `${sign}${formatDurationMs(Math.abs(deltaMs))}`;
}
