/** Per-ds extraSession KV in localStorage. Author: kejiqing */

const STORAGE_KEY = "claw-extra-session-by-ds";

export type ExtraSessionKv = Record<string, string>;

function readMap(): Record<string, ExtraSessionKv> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return {};
    return parsed as Record<string, ExtraSessionKv>;
  } catch {
    return {};
  }
}

function writeMap(map: Record<string, ExtraSessionKv>): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(map));
  } catch {
    /* ignore */
  }
}

export function loadExtraSessionKvForDs(dsId: number): ExtraSessionKv {
  const map = readMap();
  const kv = map[String(dsId)];
  return kv && typeof kv === "object" ? { ...kv } : {};
}

export function saveExtraSessionKvForDs(dsId: number, kv: ExtraSessionKv): void {
  const map = readMap();
  map[String(dsId)] = { ...kv };
  writeMap(map);
}

export function emptyFieldsRecord(fields: string[]): ExtraSessionKv {
  const out: ExtraSessionKv = {};
  for (const f of fields) {
    out[f] = "";
  }
  return out;
}

export function mergeFieldsWithKv(
  fields: string[],
  kv: ExtraSessionKv
): ExtraSessionKv {
  const out = emptyFieldsRecord(fields);
  for (const f of fields) {
    if (Object.prototype.hasOwnProperty.call(kv, f) && typeof kv[f] === "string") {
      out[f] = kv[f];
    }
  }
  return out;
}

/** Map turn snapshot `extraSession` into composer fields (defined keys only). Author: kejiqing */
export function kvFromExtraSession(
  fields: string[],
  extra: Record<string, unknown> | null | undefined
): ExtraSessionKv {
  const out = emptyFieldsRecord(fields);
  if (!extra || typeof extra !== "object") return out;
  for (const f of fields) {
    const v = extra[f];
    if (typeof v === "string") out[f] = v;
  }
  return out;
}
