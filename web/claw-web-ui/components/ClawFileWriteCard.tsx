"use client";

import type { ClawFileWritePayload, ClawToolEnvelope } from "@/lib/claw-tool-envelope";

type Props = {
  envelope: ClawToolEnvelope;
};

/** Granular file write / create card (structuredPatch). Author: kejiqing */
export function ClawFileWriteCard({ envelope }: Props) {
  const payload = envelope.payload as ClawFileWritePayload;
  const path = payload?.filePath ?? "—";
  const hunks = payload?.structuredPatch ?? [];

  return (
    <div
      className={`claw-tool-card claw-tool-card--file${envelope.ok ? "" : " claw-tool-card--err"}`}
      data-testid="claw-tool-file-write"
    >
      <div className="claw-tool-card-head">
        <span className="claw-tool-card-kind">{payload?.type === "create" ? "创建文件" : "写入文件"}</span>
        <code className="claw-tool-card-path" title={path}>
          {path}
        </code>
      </div>
      <p className="claw-tool-card-summary">{envelope.summary}</p>
      {hunks.length > 0 ? (
        <pre className="claw-tool-patch">
          {hunks.flatMap((h, hi) =>
            h.lines.map((line, li) => {
              const add = line.startsWith("+");
              const del = line.startsWith("-");
              const cls = add ? "claw-patch-add" : del ? "claw-patch-del" : "claw-patch-ctx";
              return (
                <div key={`${hi}-${li}`} className={cls}>
                  {line}
                </div>
              );
            }),
          )}
        </pre>
      ) : payload?.content ? (
        <pre className="claw-tool-patch claw-tool-patch--full">{payload.content}</pre>
      ) : null}
      {!envelope.ok && envelope.error ? (
        <p className="claw-tool-card-error">{envelope.error}</p>
      ) : null}
    </div>
  );
}
