import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { message } from "antd";
import {
  fetchPlaygroundConfig,
  proxyHttp,
  type PlaygroundConfig,
} from "../api/client";
import type { ProjectConfig, ProjectListItem } from "../types/project";
import { loadProjectConfig } from "../utils/projectConfig";

const DS_KEY = "claw-playground-ds-id";
const GW_KEY = "claw-playground-gateway-base";

interface AppContextValue {
  playground: PlaygroundConfig | null;
  gatewayBase: string;
  setGatewayBase: (v: string) => void;
  dsId: number;
  setDsId: (id: number) => void;
  projects: ProjectListItem[];
  refreshProjects: (silent?: boolean) => Promise<void>;
  projectConfig: ProjectConfig | null;
  refreshProjectConfig: () => Promise<ProjectConfig>;
  gatewayOptions: { label: string; value: string }[];
  /** From GET /healthz deployImageTag (local | release-vX.Y.Z | …). Author: kejiqing */
  gatewayImageTag: string;
}

const AppContext = createContext<AppContextValue | null>(null);

export function AppProvider({ children }: { children: ReactNode }) {
  const [playground, setPlayground] = useState<PlaygroundConfig | null>(null);
  const [gatewayBase, setGatewayBaseState] = useState("");
  const [dsId, setDsIdState] = useState(1);
  const [projects, setProjects] = useState<ProjectListItem[]>([]);
  const [projectConfig, setProjectConfig] = useState<ProjectConfig | null>(null);
  const [gatewayImageTag, setGatewayImageTag] = useState("");

  const gatewayOptions = useMemo(() => {
    if (!playground) return [];
    const tagSuffix =
      gatewayImageTag && gatewayBase
        ? ` · ${gatewayImageTag}`
        : "";
    const labelFor = (baseLabel: string, value: string) =>
      value.replace(/\/$/, "") === gatewayBase.replace(/\/$/, "")
        ? baseLabel + tagSuffix
        : baseLabel;
    const out: { label: string; value: string }[] = [];
    const def = playground.defaultGatewayBase;
    if (def) {
      out.push({
        label: labelFor(playground.defaultGatewayLabel || def, def),
        value: def,
      });
    }
    for (const p of playground.gatewayPresets || []) {
      if (p.value && p.value !== def) {
        out.push({ label: labelFor(p.label, p.value), value: p.value });
      }
    }
    return out;
  }, [playground, gatewayBase, gatewayImageTag]);

  useEffect(() => {
    fetchPlaygroundConfig()
      .then((cfg) => {
        setPlayground(cfg);
        let saved = "";
        try {
          saved = localStorage.getItem(GW_KEY) || "";
        } catch {
          /* ignore */
        }
        const values = [
          cfg.defaultGatewayBase,
          ...(cfg.gatewayPresets || []).map((p) => p.value),
        ].filter(Boolean);
        if (saved && values.includes(saved)) setGatewayBaseState(saved);
        else if (cfg.defaultGatewayBase) setGatewayBaseState(cfg.defaultGatewayBase);
      })
      .catch((e) => message.error(String((e as Error).message)));
  }, []);

  const setGatewayBase = useCallback((v: string) => {
    setGatewayBaseState(v);
    try {
      localStorage.setItem(GW_KEY, v);
    } catch {
      /* ignore */
    }
  }, []);

  const setDsId = useCallback((id: number) => {
    setDsIdState(id);
    try {
      localStorage.setItem(DS_KEY, String(id));
    } catch {
      /* ignore */
    }
  }, []);

  const refreshProjectConfig = useCallback(async () => {
    if (!gatewayBase) throw new Error("未选择网关");
    const cfg = await loadProjectConfig(gatewayBase, dsId);
    setProjectConfig(cfg);
    return cfg;
  }, [gatewayBase, dsId]);

  const refreshProjects = useCallback(
    async (silent?: boolean) => {
      if (!gatewayBase) return;
      try {
        const hz = await proxyHttp<{
          deployImageTag?: string;
        }>(gatewayBase, "GET", "/healthz");
        setGatewayImageTag((hz.deployImageTag || "").trim());
      } catch {
        setGatewayImageTag("");
      }
      let list: ProjectListItem[] = [];
      try {
        const r = await proxyHttp<{ projects: ProjectListItem[] }>(
          gatewayBase,
          "GET",
          "/v1/projects"
        );
        list = r.projects || [];
      } catch (e) {
        if (!silent) {
          message.warning(
            `GET /v1/projects 失败，回退 healthz: ${(e as Error).message}`
          );
        }
        const h = await proxyHttp<{
          projectsGitMirror?: { dsWorkspaces?: ProjectListItem[] };
        }>(gatewayBase, "GET", "/healthz");
        list = h.projectsGitMirror?.dsWorkspaces || [];
      }
      list.sort((a, b) => a.dsId - b.dsId);
      setProjects(list);
      const saved = parseInt(localStorage.getItem(DS_KEY) || "", 10);
      const cur = dsId;
      if (list.length && !list.some((p) => p.dsId === cur)) {
        const pick =
          list.find((p) => p.dsId === saved) ||
          list.find((p) => p.environmentPrepared) ||
          list[0];
        setDsId(pick.dsId);
      } else if (list.length && Number.isFinite(saved) && list.some((p) => p.dsId === saved)) {
        setDsIdState(saved);
      }
      if (!silent) message.success(`已加载 ${list.length} 个项目`);
    },
    [gatewayBase, dsId, setDsId]
  );

  useEffect(() => {
    if (!gatewayBase) return;
    refreshProjects(true).catch(() => {});
  }, [gatewayBase, refreshProjects]);

  useEffect(() => {
    if (!gatewayBase) return;
    refreshProjectConfig().catch(() => setProjectConfig(null));
  }, [gatewayBase, dsId, refreshProjectConfig]);

  const value: AppContextValue = {
    playground,
    gatewayBase,
    setGatewayBase,
    dsId,
    setDsId,
    projects,
    refreshProjects,
    projectConfig,
    refreshProjectConfig,
    gatewayOptions,
    gatewayImageTag,
  };

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): AppContextValue {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp outside AppProvider");
  return ctx;
}
