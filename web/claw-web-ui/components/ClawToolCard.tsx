"use client";

import type { ClawToolEnvelope } from "@/lib/claw-tool-envelope";
import { ClawFileWriteCard } from "./ClawFileWriteCard";

type Props = {
  envelope: ClawToolEnvelope;
};

/** Route payloadKind → granular card. Author: kejiqing */
export function ClawToolCard({ envelope }: Props) {
  if (envelope.payloadKind === "file_write" || envelope.payloadKind === "file_edit") {
    return <ClawFileWriteCard envelope={envelope} />;
  }

  return (
    <div
      className={`claw-tool-card claw-tool-card--generic${envelope.ok ? "" : " claw-tool-card--err"}`}
      data-testid="claw-tool-generic"
    >
      <div className="claw-tool-card-head">
        <span className="claw-tool-card-kind">{envelope.toolName}</span>
        <span className="claw-tool-card-badge">{envelope.payloadKind}</span>
      </div>
      <p className="claw-tool-card-summary">{envelope.summary}</p>
      <pre className="claw-tool-json">{JSON.stringify(envelope.payload, null, 2)}</pre>
      {!envelope.ok && envelope.error ? (
        <p className="claw-tool-card-error">{envelope.error}</p>
      ) : null}
    </div>
  );
}
