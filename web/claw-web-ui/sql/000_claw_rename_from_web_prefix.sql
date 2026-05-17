-- One-time: claw_web_* → claw_* (existing DBs before table rename). Author: kejiqing

DO $migrate$
BEGIN
  IF to_regclass('public.claw_web_users') IS NOT NULL
     AND to_regclass('public.claw_users') IS NULL THEN
    ALTER TABLE claw_web_users RENAME TO claw_users;
  END IF;

  IF to_regclass('public.claw_web_projects') IS NOT NULL
     AND to_regclass('public.claw_projects') IS NULL THEN
    ALTER TABLE claw_web_projects RENAME TO claw_projects;
  END IF;

  IF to_regclass('public.claw_web_user_projects') IS NOT NULL
     AND to_regclass('public.claw_user_projects') IS NULL THEN
    ALTER TABLE claw_web_user_projects RENAME TO claw_user_projects;
  END IF;

  IF to_regclass('public.claw_web_project_state') IS NOT NULL
     AND to_regclass('public.claw_project_state') IS NULL THEN
    ALTER TABLE claw_web_project_state RENAME TO claw_project_state;
  END IF;

  IF to_regclass('public.claw_web_sessions') IS NOT NULL
     AND to_regclass('public.claw_sessions') IS NULL THEN
    ALTER TABLE claw_web_sessions RENAME TO claw_sessions;
  END IF;

  IF to_regclass('public.claw_web_tunnels') IS NOT NULL
     AND to_regclass('public.claw_tunnels') IS NULL THEN
    ALTER TABLE claw_web_tunnels RENAME TO claw_tunnels;
  END IF;

  IF to_regclass('public.claw_web_messages') IS NOT NULL
     AND to_regclass('public.claw_messages') IS NULL THEN
    ALTER TABLE claw_web_messages RENAME TO claw_messages;
  END IF;
END $migrate$;
