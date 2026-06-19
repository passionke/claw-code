/** LLM provider presets from CSV metadata. Author: kejiqing */

import providersCsv from "../data/llm-providers.csv?raw";

export const LLM_PROVIDER_CUSTOM_ID = "custom";

export interface LlmProviderPreset {
  presetId: string;
  providerLabel: string;
  baseModelUrl: string;
  modelId: string;
  displayName: string;
}

function parseCsvLine(line: string): string[] {
  const out: string[] = [];
  let cur = "";
  let inQuotes = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (ch === '"') {
      if (inQuotes && line[i + 1] === '"') {
        cur += '"';
        i++;
      } else {
        inQuotes = !inQuotes;
      }
      continue;
    }
    if (ch === "," && !inQuotes) {
      out.push(cur);
      cur = "";
      continue;
    }
    cur += ch;
  }
  out.push(cur);
  return out;
}

/** Parse bundled `llm-providers.csv` (header row required). */
export function parseLlmProviderCsv(csv: string): LlmProviderPreset[] {
  const lines = csv
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith("#"));
  if (lines.length < 2) return [];
  const header = parseCsvLine(lines[0]);
  const idx = (name: string) => header.indexOf(name);
  const cols = {
    presetId: idx("preset_id"),
    providerLabel: idx("provider_label"),
    baseModelUrl: idx("base_model_url"),
    modelId: idx("model_id"),
    displayName: idx("display_name"),
  };
  if (Object.values(cols).some((i) => i < 0)) return [];

  const out: LlmProviderPreset[] = [];
  for (const line of lines.slice(1)) {
    const cells = parseCsvLine(line);
    const presetId = (cells[cols.presetId] || "").trim();
    const baseModelUrl = (cells[cols.baseModelUrl] || "").trim();
    const modelId = (cells[cols.modelId] || "").trim();
    if (!presetId || !baseModelUrl || !modelId) continue;
    out.push({
      presetId,
      providerLabel: (cells[cols.providerLabel] || "").trim() || presetId,
      baseModelUrl,
      modelId,
      displayName: (cells[cols.displayName] || "").trim() || modelId,
    });
  }
  return out;
}

export const LLM_PROVIDER_PRESETS: LlmProviderPreset[] = parseLlmProviderCsv(providersCsv);

export function llmPresetSelectLabel(p: LlmProviderPreset): string {
  return `${p.providerLabel} · ${p.modelId}`;
}

export function matchLlmPreset(
  presets: LlmProviderPreset[],
  baseModelUrl: string,
  modelId: string
): LlmProviderPreset | undefined {
  const base = baseModelUrl.trim().replace(/\/$/, "");
  const model = modelId.trim();
  return presets.find(
    (p) => p.baseModelUrl.replace(/\/$/, "") === base && p.modelId === model
  );
}

export function findLlmPresetByEndpoint(
  baseModelUrl: string,
  modelId: string
): LlmProviderPreset | undefined {
  return matchLlmPreset(LLM_PROVIDER_PRESETS, baseModelUrl, modelId);
}

export function groupLlmPresetsByProvider(
  presets: LlmProviderPreset[]
): { providerLabel: string; presets: LlmProviderPreset[] }[] {
  const map = new Map<string, LlmProviderPreset[]>();
  for (const p of presets) {
    const list = map.get(p.providerLabel) || [];
    list.push(p);
    map.set(p.providerLabel, list);
  }
  return [...map.entries()].map(([providerLabel, items]) => ({
    providerLabel,
    presets: items,
  }));
}
