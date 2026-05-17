"use client";

import { useSyncExternalStore } from "react";
import { readStoredThreadId } from "@/lib/claw-config";

function subscribe(onStoreChange: () => void) {
  window.addEventListener("storage", onStoreChange);
  return () => window.removeEventListener("storage", onStoreChange);
}

/** Client-only claw-session-id (= AG-UI threadId, gateway header). Author: kejiqing */
export function useClawSessionId(): string {
  return useSyncExternalStore(
    subscribe,
    () => readStoredThreadId() ?? "",
    () => "",
  );
}
