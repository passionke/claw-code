-- Inbox message threading + from address. Author: kejiqing
ALTER TABLE session_inbox_messages
  ADD COLUMN IF NOT EXISTS from_address TEXT;
ALTER TABLE session_inbox_messages
  ADD COLUMN IF NOT EXISTS in_reply_to TEXT;
ALTER TABLE session_inbox_messages
  ADD COLUMN IF NOT EXISTS references_json JSONB;
