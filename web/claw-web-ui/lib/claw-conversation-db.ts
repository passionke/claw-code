/** Server: conversation CRUD in PostgreSQL (scoped by user_id). Author: kejiqing */

import type {
  ClawSessionRecord,
  ClawSessionSummary,
  ClawTunnelMessage,
  ClawTunnelRecord,
} from "@/lib/claw-conversation-types";
import { deriveTitle } from "@/lib/claw-conversation-types";
import { generateSessionTitle } from "@/lib/claw-session-title-llm";
import { withPg } from "@/lib/claw-pg";

type RowMsg = {
  tunnel_id: string;
  message_id: string;
  role: string;
  content: string;
  created_at_ms: string;
  run_id: string | null;
};

type RowTunnel = {
  tunnel_id: string;
  run_id: string | null;
  status: string;
  user_preview: string;
  error_preview: string | null;
  started_at_ms: string;
  finished_at_ms: string | null;
};

function mapMessage(row: RowMsg): ClawTunnelMessage {
  return {
    tunnelId: row.tunnel_id,
    messageId: row.message_id,
    role: row.role as "user" | "assistant",
    content: row.content,
    createdAtMs: Number(row.created_at_ms),
    runId: row.run_id,
  };
}

function mapTunnel(row: RowTunnel): ClawTunnelRecord {
  return {
    tunnelId: row.tunnel_id,
    runId: row.run_id,
    status: row.status as ClawTunnelRecord["status"],
    userPreview: row.user_preview,
    errorPreview: row.error_preview,
    startedAtMs: Number(row.started_at_ms),
    finishedAtMs: row.finished_at_ms != null ? Number(row.finished_at_ms) : null,
  };
}

function tunnelAggregates(messages: ClawTunnelMessage[]): Map<
  string,
  {
    userPreview: string;
    runId: string | null;
    startedAtMs: number;
    finishedAtMs: number;
  }
> {
  const map = new Map<
    string,
    { userPreview: string; runId: string | null; startedAtMs: number; finishedAtMs: number }
  >();
  for (const m of messages) {
    const ts = m.createdAtMs || Date.now();
    let agg = map.get(m.tunnelId);
    if (!agg) {
      agg = {
        userPreview: m.role === "user" ? m.content.slice(0, 240) : "",
        runId: m.runId ?? null,
        startedAtMs: ts,
        finishedAtMs: ts,
      };
      map.set(m.tunnelId, agg);
    } else {
      if (m.role === "user" && !agg.userPreview) {
        agg.userPreview = m.content.slice(0, 240);
      }
      if (m.runId && !agg.runId) agg.runId = m.runId;
      agg.startedAtMs = Math.min(agg.startedAtMs, ts);
      agg.finishedAtMs = Math.max(agg.finishedAtMs, ts);
    }
  }
  return map;
}

export async function listSessionSummaries(
  userId: string,
  projectId: string,
): Promise<{
  activeSessionId: string | null;
  sessions: ClawSessionSummary[];
}> {
  return withPg(async (client) => {
    await client.query(
      `INSERT INTO claw_project_state (user_id, project_id, active_session_id, updated_at_ms)
       VALUES ($1, $2, NULL, $3)
       ON CONFLICT (user_id, project_id) DO NOTHING`,
      [userId, projectId, Date.now()],
    );
    const state = await client.query<{ active_session_id: string | null }>(
      `SELECT active_session_id FROM claw_project_state
       WHERE user_id = $1 AND project_id = $2`,
      [userId, projectId],
    );
    const rows = await client.query<{
      session_id: string;
      title: string;
      created_at_ms: string;
      updated_at_ms: string;
      archived_at_ms: string | null;
    }>(
      `SELECT session_id, title, created_at_ms, updated_at_ms, archived_at_ms
       FROM claw_sessions
       WHERE user_id = $1 AND project_id = $2 AND archived_at_ms IS NULL
       ORDER BY updated_at_ms DESC`,
      [userId, projectId],
    );
    return {
      activeSessionId: state.rows[0]?.active_session_id ?? null,
      sessions: rows.rows.map((r) => ({
        projectId,
        sessionId: r.session_id,
        title: r.title,
        createdAtMs: Number(r.created_at_ms),
        updatedAtMs: Number(r.updated_at_ms),
        archivedAtMs: r.archived_at_ms != null ? Number(r.archived_at_ms) : null,
      })),
    };
  });
}

export async function getSessionRecord(
  userId: string,
  projectId: string,
  sessionId: string,
): Promise<ClawSessionRecord | null> {
  return withPg(async (client) => {
    const sess = await client.query<{
      title: string;
      created_at_ms: string;
      updated_at_ms: string;
    }>(
      `SELECT title, created_at_ms, updated_at_ms FROM claw_sessions
       WHERE user_id = $1 AND project_id = $2 AND session_id = $3`,
      [userId, projectId, sessionId],
    );
    if (sess.rowCount === 0) return null;
    const r = sess.rows[0];
    const tunnels = await client.query<RowTunnel>(
      `SELECT tunnel_id, run_id, status, user_preview, error_preview, started_at_ms, finished_at_ms
       FROM claw_tunnels
       WHERE user_id = $1 AND project_id = $2 AND session_id = $3
       ORDER BY started_at_ms ASC`,
      [userId, projectId, sessionId],
    );
    const msgs = await client.query<RowMsg>(
      `SELECT tunnel_id, message_id, role, content, created_at_ms, run_id
       FROM claw_messages
       WHERE user_id = $1 AND project_id = $2 AND session_id = $3
       ORDER BY seq ASC`,
      [userId, projectId, sessionId],
    );
    return {
      projectId,
      sessionId,
      title: r.title,
      createdAtMs: Number(r.created_at_ms),
      updatedAtMs: Number(r.updated_at_ms),
      tunnels: tunnels.rows.map(mapTunnel),
      messages: msgs.rows.map(mapMessage),
    };
  });
}

export async function createSessionRecord(
  userId: string,
  projectId: string,
  sessionId: string,
  messages: ClawTunnelMessage[] = [],
): Promise<ClawSessionRecord> {
  return withPg(async (client) => {
    const now = Date.now();
    const title = messages.length > 0 ? deriveTitle(messages) : "新对话";
    await client.query("BEGIN");
    try {
      await client.query(
        `INSERT INTO claw_project_state (user_id, project_id, active_session_id, updated_at_ms)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id, project_id) DO UPDATE SET active_session_id = $3, updated_at_ms = $4`,
        [userId, projectId, sessionId, now],
      );
      await client.query(
        `INSERT INTO claw_sessions (user_id, project_id, session_id, title, created_at_ms, updated_at_ms)
         VALUES ($1, $2, $3, $4, $5, $5)
         ON CONFLICT (user_id, project_id, session_id) DO UPDATE SET title = $4, updated_at_ms = $5`,
        [userId, projectId, sessionId, title, now],
      );
      await replaceMessages(client, userId, projectId, sessionId, messages);
      await client.query("COMMIT");
    } catch (e) {
      await client.query("ROLLBACK");
      throw e;
    }
    return (await getSessionRecord(userId, projectId, sessionId))!;
  });
}

async function replaceMessages(
  client: import("pg").PoolClient,
  userId: string,
  projectId: string,
  sessionId: string,
  messages: ClawTunnelMessage[],
): Promise<void> {
  await client.query(
    `DELETE FROM claw_tunnels WHERE user_id = $1 AND project_id = $2 AND session_id = $3`,
    [userId, projectId, sessionId],
  );
  const tunnels = tunnelAggregates(messages);
  for (const [tunnelId, agg] of tunnels) {
    await client.query(
      `INSERT INTO claw_tunnels (
         user_id, project_id, session_id, tunnel_id, run_id, status,
         user_preview, started_at_ms, finished_at_ms
       ) VALUES ($1, $2, $3, $4, $5, 'completed', $6, $7, $8)`,
      [
        userId,
        projectId,
        sessionId,
        tunnelId,
        agg.runId,
        agg.userPreview,
        agg.startedAtMs,
        agg.finishedAtMs,
      ],
    );
  }
  let seq = 0;
  for (const m of messages) {
    await client.query(
      `INSERT INTO claw_messages (
         user_id, project_id, session_id, tunnel_id, message_id, role, content, seq, created_at_ms, run_id
       ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)`,
      [
        userId,
        projectId,
        sessionId,
        m.tunnelId,
        m.messageId,
        m.role,
        m.content,
        seq,
        m.createdAtMs || Date.now(),
        m.runId ?? null,
      ],
    );
    seq += 1;
  }
}

export async function saveSessionMessages(
  userId: string,
  projectId: string,
  sessionId: string,
  messages: ClawTunnelMessage[],
): Promise<ClawSessionRecord> {
  return withPg(async (client) => {
    const now = Date.now();
    const hasUser = messages.some((m) => m.role === "user" && m.content.trim());
    const hasAssistant = messages.some((m) => m.role === "assistant");
    await client.query("BEGIN");
    try {
      const exists = await client.query<{ title: string }>(
        `SELECT title FROM claw_sessions
         WHERE user_id = $1 AND project_id = $2 AND session_id = $3`,
        [userId, projectId, sessionId],
      );
      const prevTitle = exists.rows[0]?.title ?? "新对话";
      let title = prevTitle;
      if (hasUser) {
        if (hasAssistant) {
          title = await generateSessionTitle(messages);
        } else {
          title = deriveTitle(messages);
        }
      } else if (exists.rowCount === 0) {
        title = "新对话";
      }
      if (exists.rowCount === 0) {
        await client.query(
          `INSERT INTO claw_sessions (user_id, project_id, session_id, title, created_at_ms, updated_at_ms)
           VALUES ($1, $2, $3, $4, $5, $5)`,
          [userId, projectId, sessionId, title, now],
        );
      } else {
        await client.query(
          `UPDATE claw_sessions SET title = $4, updated_at_ms = $5
           WHERE user_id = $1 AND project_id = $2 AND session_id = $3`,
          [userId, projectId, sessionId, title, now],
        );
      }
      await replaceMessages(client, userId, projectId, sessionId, messages);
      await client.query(
        `INSERT INTO claw_project_state (user_id, project_id, active_session_id, updated_at_ms)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id, project_id) DO UPDATE SET active_session_id = $3, updated_at_ms = $4`,
        [userId, projectId, sessionId, now],
      );
      await client.query("COMMIT");
    } catch (e) {
      await client.query("ROLLBACK");
      throw e;
    }
    return (await getSessionRecord(userId, projectId, sessionId))!;
  });
}

export async function archiveSessionRecord(
  userId: string,
  projectId: string,
  sessionId: string,
): Promise<void> {
  await withPg(async (client) => {
    const now = Date.now();
    const res = await client.query(
      `UPDATE claw_sessions SET archived_at_ms = $4, updated_at_ms = $4
       WHERE user_id = $1 AND project_id = $2 AND session_id = $3`,
      [userId, projectId, sessionId, now],
    );
    if (res.rowCount === 0) {
      throw new Error("session not found");
    }
    const state = await client.query<{ active_session_id: string | null }>(
      `SELECT active_session_id FROM claw_project_state
       WHERE user_id = $1 AND project_id = $2`,
      [userId, projectId],
    );
    if (state.rows[0]?.active_session_id === sessionId) {
      await client.query(
        `UPDATE claw_project_state SET active_session_id = NULL, updated_at_ms = $3
         WHERE user_id = $1 AND project_id = $2`,
        [userId, projectId, now],
      );
    }
  });
}

export async function deleteSessionRecord(
  userId: string,
  projectId: string,
  sessionId: string,
): Promise<void> {
  await withPg(async (client) => {
    const now = Date.now();
    await client.query("BEGIN");
    try {
      const state = await client.query<{ active_session_id: string | null }>(
        `SELECT active_session_id FROM claw_project_state
         WHERE user_id = $1 AND project_id = $2`,
        [userId, projectId],
      );
      if (state.rows[0]?.active_session_id === sessionId) {
        await client.query(
          `UPDATE claw_project_state SET active_session_id = NULL, updated_at_ms = $3
           WHERE user_id = $1 AND project_id = $2`,
          [userId, projectId, now],
        );
      }
      const del = await client.query(
        `DELETE FROM claw_sessions
         WHERE user_id = $1 AND project_id = $2 AND session_id = $3`,
        [userId, projectId, sessionId],
      );
      if (del.rowCount === 0) {
        throw new Error("session not found");
      }
      await client.query("COMMIT");
    } catch (e) {
      await client.query("ROLLBACK");
      throw e;
    }
  });
}

export async function setActiveSession(
  userId: string,
  projectId: string,
  sessionId: string,
): Promise<void> {
  await withPg(async (client) => {
    const now = Date.now();
    await client.query(
      `INSERT INTO claw_project_state (user_id, project_id, active_session_id, updated_at_ms)
       VALUES ($1, $2, $3, $4)
       ON CONFLICT (user_id, project_id) DO UPDATE SET active_session_id = $3, updated_at_ms = $4`,
      [userId, projectId, sessionId, now],
    );
  });
}

export async function migrateProject(
  userId: string,
  projectId: string,
  activeSessionId: string | null,
  sessions: ClawSessionRecord[],
): Promise<void> {
  await withPg(async (client) => {
    await client.query("BEGIN");
    try {
      const now = Date.now();
      for (const s of sessions) {
        await client.query(
          `INSERT INTO claw_sessions (user_id, project_id, session_id, title, created_at_ms, updated_at_ms)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (user_id, project_id, session_id) DO UPDATE
             SET title = EXCLUDED.title, updated_at_ms = EXCLUDED.updated_at_ms`,
          [userId, projectId, s.sessionId, s.title, s.createdAtMs, s.updatedAtMs],
        );
        await replaceMessages(client, userId, projectId, s.sessionId, s.messages);
      }
      if (activeSessionId) {
        await client.query(
          `INSERT INTO claw_project_state (user_id, project_id, active_session_id, updated_at_ms)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (user_id, project_id) DO UPDATE SET active_session_id = $3, updated_at_ms = $4`,
          [userId, projectId, activeSessionId, now],
        );
      }
      await client.query("COMMIT");
    } catch (e) {
      await client.query("ROLLBACK");
      throw e;
    }
  });
}
