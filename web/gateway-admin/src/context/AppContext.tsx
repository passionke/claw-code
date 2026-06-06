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
  /** Apply PUT /v1/project/config response without an extra GET. */
  applyProjectConfig: (cfg: ProjectConfig) => void;
  gatewayOptions: { label: string; value: string }[];
  /** Multiple claw_pool rows with gatewayBase — else hide meaningless picker. Author: kejiqing */
  showGatewayPicker: boolean;
  /** GET /v1/pools from default gateway — turn route labels. Author: kejiqing */
  clusterPools: ListClawPoolsResponse | null;
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
    applyProjectConfig,
    gatewayOptions,
    showGatewayPicker,
    clusterPools,
    gatewayImageTag,
  };

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): AppContextValue {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp outside AppProvider");
  return ctx;
}
