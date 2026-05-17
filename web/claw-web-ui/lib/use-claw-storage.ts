"use client";

import { useEffect, useState } from "react";
import { useSyncExternalStore } from "react";
import { defaultDsId, readStoredDsId } from "@/lib/claw-config";
import { fetchConversationIndex, subscribeStore } from "@/lib/claw-conversation-client";
import { projectIdFromDsId } from "@/lib/claw-conversation-types";

/** Read dsId / active sessionId from PG index (via BFF). Author: kejiqing */
export function useClawStorageIds(): { dsId: number; threadId: string | null } {
  const dsId = useSyncExternalStore(
    subscribeStore,
    () => readStoredDsId(),
    () => defaultDsId(),
  );
  const [threadId, setThreadId] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void fetchConversationIndex(projectIdFromDsId(dsId))
      .then((d) => {
        if (!cancelled) setThreadId(d.activeSessionId);
      })
      .catch(() => {
        if (!cancelled) setThreadId(null);
      });
    return () => {
      cancelled = true;
    };
  }, [dsId]);

  return { dsId, threadId };
}
