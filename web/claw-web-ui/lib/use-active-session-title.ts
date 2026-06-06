"use client";

import { useCallback, useEffect, useState } from "react";
import { fetchConversationIndex, subscribeStore } from "@/lib/claw-conversation-client";
import { projectIdFromDsId } from "@/lib/claw-conversation-types";
import { useClawUi } from "@/components/ClawCopilotProvider";

/** Current session title for dock header. Author: kejiqing */
export function useActiveSessionTitle(): string {
  const { threadId, dsId } = useClawUi();
  const projectId = projectIdFromDsId(dsId);
  const [title, setTitle] = useState("新对话");

  const reload = useCallback(async () => {
    try {
      const data = await fetchConversationIndex(projectId);
      const row = data.sessions.find((s) => s.sessionId === threadId);
      setTitle(row?.title?.trim() || "新对话");
    } catch {
      setTitle("新对话");
    }
  }, [projectId, threadId]);

  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => subscribeStore(() => void reload()), [reload]);

  return title;
}
