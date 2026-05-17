/** Server: claw_users CRUD. Author: kejiqing */

import type { ClawWebUser, EnsureUserInput } from "@/lib/claw-user-types";
import { withPg } from "@/lib/claw-pg";

function mapUser(row: {
  user_id: string;
  tenant_id: string | null;
  display_name: string;
  email: string | null;
  status: string;
  created_at_ms: string;
  updated_at_ms: string;
  last_seen_at_ms: string | null;
}): ClawWebUser {
  return {
    userId: row.user_id,
    tenantId: row.tenant_id,
    displayName: row.display_name,
    email: row.email,
    status: row.status === "disabled" ? "disabled" : "active",
    createdAtMs: Number(row.created_at_ms),
    updatedAtMs: Number(row.updated_at_ms),
    lastSeenAtMs: row.last_seen_at_ms != null ? Number(row.last_seen_at_ms) : null,
  };
}

export async function getUserById(userId: string): Promise<ClawWebUser | null> {
  return withPg(async (client) => {
    const res = await client.query(
      `SELECT user_id, tenant_id, display_name, email, status,
              created_at_ms, updated_at_ms, last_seen_at_ms
       FROM claw_users WHERE user_id = $1`,
      [userId],
    );
    if (res.rowCount === 0) return null;
    return mapUser(res.rows[0]);
  });
}

/** Insert or touch last_seen; rejects disabled users. Author: kejiqing */
export async function ensureUser(input: EnsureUserInput): Promise<ClawWebUser> {
  return withPg(async (client) => {
    const now = Date.now();
    const displayName = input.displayName?.trim() || input.userId;
    const email = input.email?.trim() || null;
    await client.query(
      `INSERT INTO claw_users (
         user_id, tenant_id, display_name, email, status,
         created_at_ms, updated_at_ms, last_seen_at_ms
       ) VALUES ($1, $2, $3, $4, 'active', $5, $5, $5)
       ON CONFLICT (user_id) DO UPDATE SET
         tenant_id = COALESCE(EXCLUDED.tenant_id, claw_users.tenant_id),
         display_name = CASE
           WHEN EXCLUDED.display_name <> '' THEN EXCLUDED.display_name
           ELSE claw_users.display_name
         END,
         email = COALESCE(EXCLUDED.email, claw_users.email),
         updated_at_ms = $5,
         last_seen_at_ms = $5`,
      [input.userId, input.tenantId ?? null, displayName, email, now],
    );
    const user = await getUserById(input.userId);
    if (!user) {
      throw new Error(`user upsert failed: ${input.userId}`);
    }
    if (user.status === "disabled") {
      throw new Error(`user disabled: ${input.userId}`);
    }
    return user;
  });
}
