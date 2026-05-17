/** Claw Web user row (maps to L5 JWT `sub` / `tenant_id`). Author: kejiqing */

export type ClawWebUser = {
  userId: string;
  tenantId: string | null;
  displayName: string;
  email: string | null;
  status: "active" | "disabled";
  createdAtMs: number;
  updatedAtMs: number;
  lastSeenAtMs: number | null;
};

export type EnsureUserInput = {
  userId: string;
  tenantId?: string | null;
  displayName?: string;
  email?: string | null;
};
