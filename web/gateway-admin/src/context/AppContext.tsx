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
import { normalizeGatewayBase } from "../utils/gatewayBase";
import { loadProjectConfig } from "../utils/projectConfig";

const PROJ_KEY = "claw-playground-proj-id";
const LEGACY_DS_KEY = "claw-playground-ds-id";
const GW_KEY = "claw-playground-gateway-base";

function readSavedProjId(): number | null {
  try {
    const raw =
      localStorage.getItem(PROJ_KEY) || localStorage.getItem(LEGACY_DS_KEY) || "";
    const n = parseInt(raw, 10);
    return Number.isFinite(n) ? n : null;
  } catch {
    return null;
  }
}

function defaultGatewayBase(playground: PlaygroundConfig | null): string {
  const def = playground?.defaultGatewayBase?.trim();
  return def ? normalizeGatewayBase(def) : "";
}

interface AppContextValue {
  playground: PlaygroundConfig | null;
  gatewayBase: string;
  setGatewayBase: (v: string) => void;
  projId: number;
  setProjId: (id: number) => void;
  projects: ProjectListItem[];
  refreshProjects: (silent?: boolean) => Promise<void>;
  projectConfig: ProjectConfig | null;
  refreshProjectConfig: () => Promise<ProjectConfig>;
  /** Apply PUT /v1/project/config response without an extra GET. */
  applyProjectConfig: (cfg: ProjectConfig) => void;
  gatewayOptions: { label: string; value: string }[];
  /** From GET /healthz deployImageTag (local | release-vX.Y.Z | …). Author: kejiqing */
  gatewayImageTag: string;
}

const AppContext = createContext<AppContextValue | null>(null);

export function AppProvider({ children }: { children: ReactNode }) {
  const [playground, setPlayground] = useState<PlaygroundConfig | null>(null);
  const [gatewayBase, setGatewayBaseState] = useState("");
  const [projId, setProjIdState] = useState(() => readSavedProjId() ?? 1);
  const [projects, setProjects] = useState<ProjectListItem[]>([]);
  const [projectConfig, setProjectConfig] = useState<ProjectConfig | null>(null);
  const [gatewayImageTag, setGatewayImageTag] = useState("");

  const gatewayOptions = useMemo(() => {
    const def = defaultGatewayBase(playground);
    if (!def) return [];
    const tagSuffix =
      gatewayImageTag && gatewayBase ? ` · ${gatewayImageTag}` : "";
    let label = playground?.defaultGatewayLabel || `本机 · ${new URL(def).host}`;
    if (gatewayBase && normalizeGatewayBase(gatewayBase) === def) {
      label += tagSuffix;
    }
    return [{ value: def, label }];
  }, [playground, gatewayBase, gatewayImageTag]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const cfg = await fetchPlaygroundConfig();
        if (cancelled) return;
        setPlayground(cfg);

        let saved = "";
        try {
          saved = localStorage.getItem(GW_KEY) || "";
        } catch {
          /* ignore */
        }
        const fallback = defaultGatewayBase(cfg);
        const savedNorm = normalizeGatewayBase(saved);
        if (savedNorm && fallback && savedNorm === fallback) {
          setGatewayBaseState(savedNorm);
        } else if (fallback) {
          setGatewayBaseState(fallback);
        }
      } catch (e) {
        if (!cancelled) message.error(String((e as Error).message));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const setGatewayBase = useCallback((v: string) => {
    setGatewayBaseState(v);
    try {
      localStorage.setItem(GW_KEY, v);
    } catch {
      /* ignore */
    }
  }, []);

  const setProjId = useCallback((id: number) => {
    setProjIdState(id);
    try {
      localStorage.setItem(PROJ_KEY, String(id));
    } catch {
      /* ignore */
    }
  }, []);

  const refreshProjectConfig = useCallback(async () => {
    if (!gatewayBase) throw new Error("未选择网关");
    const cfg = await loadProjectConfig(gatewayBase, projId);
    setProjectConfig(cfg);
    return cfg;
  }, [gatewayBase, projId]);

  const applyProjectConfig = useCallback((cfg: ProjectConfig) => {
    setProjectConfig(cfg);
  }, []);

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
          projectsGitMirror?: {
            projWorkspaces?: ProjectListItem[];
            dsWorkspaces?: ProjectListItem[];
          };
        }>(gatewayBase, "GET", "/healthz");
        list =
          h.projectsGitMirror?.projWorkspaces ||
          h.projectsGitMirror?.dsWorkspaces ||
          [];
      }
      list.sort((a, b) => a.projId - b.projId);
      setProjects(list);
      const saved = readSavedProjId();
      const cur = projId;
      // Only remap when current selection is missing from the list (do not
      // clobber an explicit setProjId / URL ?projId=). Author: kejiqing
      if (list.length && !list.some((p) => p.projId === cur)) {
        const pick =
          (saved != null ? list.find((p) => p.projId === saved) : undefined) ||
          list.find((p) => p.environmentPrepared) ||
          list[0];
        setProjId(pick.projId);
      }
      if (!silent) message.success(`已加载 ${list.length} 个项目`);
    },
    [gatewayBase, projId, setProjId]
  );

  useEffect(() => {
    if (!gatewayBase) return;
    refreshProjects(true).catch(() => {});
  }, [gatewayBase, refreshProjects]);

  useEffect(() => {
    if (!gatewayBase) return;
    refreshProjectConfig().catch(() => setProjectConfig(null));
  }, [gatewayBase, projId, refreshProjectConfig]);

  const value: AppContextValue = {
    playground,
    gatewayBase,
    setGatewayBase,
    projId,
    setProjId,
    projects,
    refreshProjects,
    projectConfig,
    refreshProjectConfig,
    applyProjectConfig,
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
