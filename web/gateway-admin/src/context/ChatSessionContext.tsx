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
import { tapLiveFromClawTapSettings } from "../utils/claudeTap";
import type { GlobalSettingsResponse } from "../types/globalSettings";
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
      const json = await proxyHttp<GlobalSettingsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/global-settings"
      );
      const { tapLiveBase: base, tapLiveTemplate: template } =
        tapLiveFromClawTapSettings(json.clawTap);
      setTapLiveBase(base);
      setTapLiveTemplate(template);
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
