import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { proxyHttp } from "../api/client";
import { normalizeClaudeTapFromHealthz } from "../utils/claudeTap";
import { useApp } from "./AppContext";

const STORE_KEY = "claw-playground-store-id";
const ORG_KEY = "claw-playground-org-id";

interface ChatSessionContextValue {
  storeId: string;
  setStoreId: (v: string) => void;
  orgId: string;
  setOrgId: (v: string) => void;
  tapLiveBase: string;
  tapLiveTemplate: string;
  refreshTapLive: () => Promise<void>;
}

const ChatSessionContext = createContext<ChatSessionContextValue | null>(null);

export function ChatSessionProvider({ children }: { children: ReactNode }) {
  const { gatewayBase } = useApp();
  const [storeId, setStoreIdState] = useState("");
  const [orgId, setOrgIdState] = useState("");
  const [tapLiveBase, setTapLiveBase] = useState("");
  const [tapLiveTemplate, setTapLiveTemplate] = useState("");

  useEffect(() => {
    try {
      setStoreIdState(localStorage.getItem(STORE_KEY) || "");
      setOrgIdState(localStorage.getItem(ORG_KEY) || "");
    } catch {
      /* ignore */
    }
  }, []);

  const setStoreId = useCallback((v: string) => {
    setStoreIdState(v);
    try {
      localStorage.setItem(STORE_KEY, v);
    } catch {
      /* ignore */
    }
  }, []);

  const setOrgId = useCallback((v: string) => {
    setOrgIdState(v);
    try {
      localStorage.setItem(ORG_KEY, v);
    } catch {
      /* ignore */
    }
  }, []);

  const refreshTapLive = useCallback(async () => {
    if (!gatewayBase) {
      setTapLiveBase("");
      setTapLiveTemplate("");
      return;
    }
    try {
      const json = await proxyHttp<{
        claudeTap?: { publicLiveBaseUrl?: string; liveSessionUrlTemplate?: string };
      }>(gatewayBase, "GET", "/healthz");
      const tap = json.claudeTap;
      if (tap?.publicLiveBaseUrl) {
        const { tapLiveBase, tapLiveTemplate } = normalizeClaudeTapFromHealthz(
          tap,
          gatewayBase
        );
        setTapLiveBase(tapLiveBase);
        setTapLiveTemplate(tapLiveTemplate);
      } else {
        setTapLiveBase("");
        setTapLiveTemplate("");
      }
    } catch {
      setTapLiveBase("");
      setTapLiveTemplate("");
    }
  }, [gatewayBase]);

  useEffect(() => {
    refreshTapLive();
  }, [refreshTapLive]);

  const value = useMemo(
    () => ({
      storeId,
      setStoreId,
      orgId,
      setOrgId,
      tapLiveBase,
      tapLiveTemplate,
      refreshTapLive,
    }),
    [storeId, setStoreId, orgId, setOrgId, tapLiveBase, tapLiveTemplate, refreshTapLive]
  );

  return (
    <ChatSessionContext.Provider value={value}>{children}</ChatSessionContext.Provider>
  );
}

export function useChatSession(): ChatSessionContextValue {
  const ctx = useContext(ChatSessionContext);
  if (!ctx) throw new Error("useChatSession outside ChatSessionProvider");
  return ctx;
}
