"use client";

import { useCopilotMessagesContext } from "@copilotkit/react-core";
import { useEffect, useRef } from "react";
import {
  copilotMessagesToStored,
  storedToCopilotMessages,
} from "@/lib/claw-copilot-messages";
import {
  fetchSession,
  notifyStoreUpdated,
  saveSessionMessagesApi,
} from "@/lib/claw-conversation-client";
import { projectIdFromDsId } from "@/lib/claw-conversation-types";
import { useClawUi } from "./ClawCopilotProvider";

/** Persist CopilotKit chat ↔ PostgreSQL (via BFF). Author: kejiqing */
export function ClawConversationSync() {
  const { threadId, dsId } = useClawUi();
  const projectId = projectIdFromDsId(dsId);
  const { messages, setMessages } = useCopilotMessagesContext();
  const hydratedFor = useRef<string | null>(null);
  const skipSave = useRef(true);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!threadId) return;
    if (hydratedFor.current === threadId) return;

    skipSave.current = true;
    void (async () => {
      const session = await fetchSession(projectId, threadId);
      const stored = session?.messages ?? [];
      if (stored.length > 0) {
        setMessages(storedToCopilotMessages(stored) as never);
      } else {
        setMessages([]);
      }
      hydratedFor.current = threadId;
      skipSave.current = false;
    })();
  }, [threadId, projectId, setMessages]);

  useEffect(() => {
    if (!threadId || skipSave.current) return;
    if (hydratedFor.current !== threadId) return;

    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      void (async () => {
        const stored = copilotMessagesToStored(messages);
        await saveSessionMessagesApi(projectId, threadId, stored);
        notifyStoreUpdated();
      })();
    }, 400);

    return () => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
    };
  }, [messages, threadId, projectId]);

  return null;
}
