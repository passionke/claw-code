"use client";

type Props = {
  tapUrl: string;
  codeServerUrl?: string;
};

export function ClawMainPanel({ tapUrl, codeServerUrl }: Props) {
  return (
    <main className="claw-main" data-testid="claw-main">
      <section className="claw-hero">
        <h1>Workspace</h1>
        <p className="claw-hero-lead">
          Agent chat lives in the <strong>right sidebar</strong>. This area is for tasks, links, and
          (later) code-server.
        </p>
      </section>

      <div className="claw-cards">
        <a className="claw-card" href={tapUrl} target="_blank" rel="noreferrer">
          <span className="claw-card-label">Live tap</span>
          <span className="claw-card-value">{tapUrl}</span>
        </a>
        <div className="claw-card claw-card-static">
          <span className="claw-card-label">Request path</span>
          <span className="claw-card-value mono">/api/copilotkit → :8090 → :8088</span>
        </div>
        {codeServerUrl && (
          <a className="claw-card" href={codeServerUrl} target="_blank" rel="noreferrer">
            <span className="claw-card-label">Files</span>
            <span className="claw-card-value">code-server (read-only)</span>
          </a>
        )}
      </div>

      {codeServerUrl && (
        <iframe
          title="code-server"
          className="claw-code-server-frame"
          data-testid="code-server-frame"
          src={codeServerUrl}
        />
      )}
    </main>
  );
}
