-- Session/turn client origin for admin vs external callers. Author: kejiqing

ALTER TABLE gateway_sessions ADD COLUMN IF NOT EXISTS client_origin TEXT;
ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS client_origin TEXT;
