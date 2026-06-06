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
  const { threadId, dsId, switching } = useClawUi();
  const projectId = projectIdFromDsId(dsId);
  const { messages, setMessages } = useCopilotMessagesContext();
  const hydratedFor = useRef<string | null>(null);
  const skipSave = useRef(true);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!threadId || switching) return;

    if (hydratedFor.current === threadId) return;

    let cancelled = false;
    skipSave.current = true;
    setMessages([]);

    void (async () => {
      try {
        const session = await fetchSession(projectId, threadId);
        if (cancelled) return;
        const stored = session?.messages ?? [];
        setMessages(
          stored.length > 0 ? (storedToCopilotMessages(stored) as never) : [],
        );
        hydratedFor.current = threadId;
        skipSave.current = false;
      } catch (e) {
        console.error("[ClawConversationSync] hydrate failed:", e);
        if (!cancelled) {
          hydratedFor.current = threadId;
          skipSave.current = false;
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [threadId, projectId, setMessages, switching]);

  useEffect(() => {
    if (!threadId || skipSave.current || switching) return;
    if (hydratedFor.current !== threadId) return;

    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      void (async () => {
        const stored = copilotMessagesToStored(messages);
        if (stored.length === 0) return;
        try {
          await saveSessionMessagesApi(projectId, threadId, stored);
          notifyStoreUpdated();
        } catch (e) {
          console.error("[ClawConversationSync] save failed:", e);
        }
      })();
    }, 400);

    return () => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
    };
  }, [messages, threadId, projectId, switching]);

  return null;
}
