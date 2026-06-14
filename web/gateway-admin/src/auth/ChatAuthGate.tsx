import { useEffect, useState, type ReactNode } from "react";
import { Spin } from "antd";
import { fetchPlaygroundConfig } from "../api/client";
import RequireAuth from "./RequireAuth";

/** When PLAYGROUND_ADMIN_CHAT_PUBLIC=0, chat routes require login like the rest of /admin. */
export default function ChatAuthGate({ children }: { children: ReactNode }) {
  const [chatPublic, setChatPublic] = useState<boolean | null>(null);

  useEffect(() => {
    fetchPlaygroundConfig()
      .then((c) => setChatPublic(c.adminChatPublic !== false))
      .catch(() => setChatPublic(true));
  }, []);

  if (chatPublic === null) {
    return (
      <div style={{ display: "flex", justifyContent: "center", padding: 80 }}>
        <Spin size="large" />
      </div>
    );
  }
  if (!chatPublic) {
    return <RequireAuth>{children}</RequireAuth>;
  }
  return <>{children}</>;
}
