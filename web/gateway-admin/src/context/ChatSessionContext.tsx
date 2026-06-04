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

interface ChatSessionContextValue {
  tapLiveBase: string;
  tapLiveTemplate: string;
  refreshTapLive: () => Promise<void>;
}

const ChatSessionContext = createContext<ChatSessionContextValue | null>(null);

export function ChatSessionProvider({ children }: { children: ReactNode }) {
  const { gatewayBase } = useApp();
  const [tapLiveBase, setTapLiveBase] = useState("");
  const [tapLiveTemplate, setTapLiveTemplate] = useState("");

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
      tapLiveBase,
      tapLiveTemplate,
      refreshTapLive,
    }),
    [tapLiveBase, tapLiveTemplate, refreshTapLive]
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
