"use client";

import { useState } from "react";
import { useClawUi } from "./ClawCopilotProvider";

export function SettingsPanel() {
  const { dsId, setDsId } = useClawUi();
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState(dsId);

  return (
    <div className="claw-settings">
      <button type="button" className="claw-settings-toggle" onClick={() => setOpen(!open)}>
        Settings
      </button>
      {open && (
        <div className="claw-settings-panel">
          <label>
            dsId
            <input
              type="number"
              min={1}
              value={draft}
              onChange={(e) => setDraft(Number.parseInt(e.target.value, 10) || 1)}
            />
          </label>
          <button
            type="button"
            onClick={() => {
              setDsId(draft);
              setOpen(false);
            }}
          >
            Apply
          </button>
        </div>
      )}
    </div>
  );
}
