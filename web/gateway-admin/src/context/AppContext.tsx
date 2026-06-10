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
import type { ListClawPoolsResponse } from "../types/pools";
import {
  allGatewayOptionValues,
  buildGatewayOptions,
  defaultGatewayFromPools,
  normalizeGatewayBase,
  shouldShowGatewayPicker,
} from "../utils/gatewayClusterOptions";
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
  /** Multiple claw_pool rows with gatewayBase — else hide meaningless picker. Author: kejiqing */
  showGatewayPicker: boolean;
  /** GET /v1/pools — shared PG registry; refreshed for gateway picker + Pool 集群. Author: kejiqing */
  clusterPools: ListClawPoolsResponse | null;
  refreshClusterPools: () => Promise<void>;
  /** From GET /healthz deployImageTag (local | release-vX.Y.Z | …). Author: kejiqing */
  gatewayImageTag: string;
}

const AppContext = createContext<AppContextValue | null>(null);

export function AppProvider({ children }: { children: ReactNode }) {
  const [playground, setPlayground] = useState<PlaygroundConfig | null>(null);
  const [gatewayBase, setGatewayBaseState] = useState("");
  const [projId, setProjIdState] = useState(1);
  const [projects, setProjects] = useState<ProjectListItem[]>([]);
  const [projectConfig, setProjectConfig] = useState<ProjectConfig | null>(null);
  const [gatewayImageTag, setGatewayImageTag] = useState("");
  const [clusterPools, setClusterPools] = useState<ListClawPoolsResponse | null>(
    null
  );

  const gatewayOptions = useMemo(() => {
    if (!playground) return [];
    return buildGatewayOptions({
      playground,
      clusterPools,
      gatewayBase,
      gatewayImageTag,
    });
  }, [playground, clusterPools, gatewayBase, gatewayImageTag]);

  const showGatewayPicker = useMemo(() => {
    if (!playground) return false;
    return shouldShowGatewayPicker(playground, clusterPools);
  }, [playground, clusterPools]);

  const refreshClusterPools = useCallback(async () => {
    const seed =
      normalizeGatewayBase(gatewayBase) ||
      normalizeGatewayBase(playground?.defaultGatewayBase || "");
    if (!seed) return;
    try {
      const pools = await proxyHttp<ListClawPoolsResponse>(seed, "GET", "/v1/pools");
      setClusterPools(pools);
    } catch {
      /* keep last snapshot — registry is best-effort for labels */
    }
  }, [gatewayBase, playground?.defaultGatewayBase]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const cfg = await fetchPlaygroundConfig();
        if (cancelled) return;
        setPlayground(cfg);

        let pools: ListClawPoolsResponse | null = null;
        const seed = cfg.defaultGatewayBase?.trim();
        if (seed) {
          try {
            pools = await proxyHttp<ListClawPoolsResponse>(seed, "GET", "/v1/pools");
          } catch {
            pools = null;
          }
        }
        if (cancelled) return;
        setClusterPools(pools);

        let saved = "";
        try {
          saved = localStorage.getItem(GW_KEY) || "";
        } catch {
          /* ignore */
        }
        const values = allGatewayOptionValues(cfg, pools);
        const savedNorm = normalizeGatewayBase(saved);
        const fallback = defaultGatewayFromPools(cfg, pools);
        if (savedNorm && values.some((v) => normalizeGatewayBase(v) === savedNorm)) {
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

  useEffect(() => {
    if (!gatewayBase && !playground?.defaultGatewayBase) return;
    void refreshClusterPools();
    const id = window.setInterval(() => void refreshClusterPools(), 30_000);
    return () => window.clearInterval(id);
  }, [gatewayBase, playground?.defaultGatewayBase, refreshClusterPools]);

  const setGatewayBase = useCallback((v: string) => {
    setGatewayBaseState(v);
    try {
      localStorage.setItem(GW_KEY, v);
    } catch {
      /* ignore */
    }
  }, []);

  // Drop selection when pool goes offline (picker is online-only). kejiqing
  useEffect(() => {
    if (!playground || !gatewayBase) return;
    const values = allGatewayOptionValues(playground, clusterPools);
    if (!values.length) return;
    const norm = normalizeGatewayBase(gatewayBase);
    if (values.some((v) => normalizeGatewayBase(v) === norm)) return;
    const fallback = defaultGatewayFromPools(playground, clusterPools);
    if (fallback) setGatewayBase(fallback);
  }, [playground, clusterPools, gatewayBase, setGatewayBase]);

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
      if (list.length && !list.some((p) => p.projId === cur)) {
        const pick =
          (saved != null ? list.find((p) => p.projId === saved) : undefined) ||
          list.find((p) => p.environmentPrepared) ||
          list[0];
        setProjId(pick.projId);
      } else if (
        list.length &&
        saved != null &&
        list.some((p) => p.projId === saved)
      ) {
        setProjIdState(saved);
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
    showGatewayPicker,
    clusterPools,
    refreshClusterPools,
    gatewayImageTag,
  };

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): AppContextValue {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp outside AppProvider");
  return ctx;
}
