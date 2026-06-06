"use client";

import { ClawConversationList } from "./ClawConversationList";

/** Right column inside agent dock (conversation picker). Author: kejiqing */
export function ClawConvRail() {
  return (
    <aside className="claw-dock-list" aria-label="对话列表">
      <ClawConversationList layout="rail" />
    </aside>
  );
}
