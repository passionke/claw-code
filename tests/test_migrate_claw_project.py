from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).parents[1] / "scripts" / "migrate-claw-project.py"
SPEC = importlib.util.spec_from_file_location("migrate_claw_project", SCRIPT)
assert SPEC and SPEC.loader
migrate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(migrate)


class MigrateClawProjectAcceptanceTests(unittest.TestCase):
    """Executable acceptance checks for the migration design. Author: kejiqing"""

    def test_cli_has_one_safe_default_path(self) -> None:
        args = migrate.parse_args(
            [
                "--proj-id",
                "10",
                "--src-gateway",
                "http://src",
                "--dst-gateway",
                "http://dst",
            ]
        )
        self.assertEqual(args.scope, "all")
        self.assertFalse(args.apply)
        self.assertFalse(args.dry_run)
        self.assertEqual(args.cluster_id, "prod-claw-01")

    def test_dry_run_never_passes_apply_to_migration_paths(self) -> None:
        argv = [
            "--proj-id",
            "10",
            "--src-gateway",
            "http://src",
            "--dst-gateway",
            "http://dst",
            "--src-database-url",
            "postgres://src",
            "--dst-database-url",
            "postgres://dst",
            "--apply",
            "--dry-run",
        ]
        with (
            mock.patch.object(migrate, "migrate_config") as config,
            mock.patch.object(migrate, "migrate_sessions") as sessions,
        ):
            self.assertEqual(migrate.main(argv), 0)
        self.assertFalse(config.call_args.kwargs["apply"])
        self.assertFalse(sessions.call_args.kwargs["apply"])

    def test_config_mapping_and_comparison_cover_every_written_field(self) -> None:
        source = {
            "claudeMd": "中文配置",
            "mcpServersJson": {"docs": {"url": "http://mcp"}},
            "rulesJson": [{"name": "rule"}],
            "skillsJson": [{"name": "skill"}],
            "skillsSourcesJson": None,
            "allowedToolsJson": ["Read"],
            "extraSessionFieldsJson": {"ticket": "string"},
            "solvePreflightJson": {"enabled": True},
            "solveOrchestrationJson": {"mode": "single"},
            "languagePipelineJson": {"language": "zh-CN"},
            "promptLimitsJson": None,
            "gitSyncJson": {"enabled": False},
            "workerIsolationJson": {"kind": "local"},
        }
        expected = migrate.build_config_put_body(source)
        destination = dict(expected)

        self.assertEqual(expected["skillsSourcesJson"], {})
        self.assertEqual(expected["promptLimitsJson"], {})
        self.assertEqual(
            expected["workerProfileJson"], source["workerIsolationJson"]
        )
        self.assertEqual(migrate.compare_configs(source, destination), [])

        for field in migrate.CONFIG_COMPARE_FIELDS:
            changed = dict(destination)
            changed[field] = {"unexpected": True}
            self.assertIn(field, migrate.compare_configs(source, changed))

    def test_config_apply_uses_create_draft_commit_activate_flow(self) -> None:
        source = {
            "contentRev": "src-rev",
            "claudeMd": "迁移配置",
            "mcpServersJson": {},
            "rulesJson": [],
            "skillsJson": [],
            "skillsSourcesJson": None,
            "allowedToolsJson": [],
            "extraSessionFieldsJson": None,
            "solvePreflightJson": None,
            "solveOrchestrationJson": None,
            "languagePipelineJson": None,
            "promptLimitsJson": None,
            "gitSyncJson": None,
            "workerIsolationJson": {"kind": "local"},
        }
        destination = migrate.build_config_put_body(source)
        calls: list[tuple[str, str, str, object]] = []
        destination_gets = 0

        def fake_http(
            method: str,
            base: str,
            path: str,
            body: object = None,
            timeout: float = 120,
        ) -> tuple[int, object]:
            nonlocal destination_gets
            del timeout
            calls.append((method, base, path, body))
            if base == "http://src":
                return 200, source
            if path == "/v1/projects" and method == "GET":
                return 200, {"projects": []}
            if path == "/v1/projects" and method == "POST":
                return 201, {"projId": 10}
            if path == "/v1/project/config/10" and method == "GET":
                destination_gets += 1
                return (404, {}) if destination_gets == 1 else (200, destination)
            if path == "/v1/project/config/10" and method == "PUT":
                self.assertEqual(body, destination)
                return 200, {}
            if path.endswith("/versions/commit"):
                return 200, {"savedContentRev": "dst-rev"}
            if path.endswith("/versions/dst-rev/activate"):
                return 200, {}
            self.fail(f"unexpected HTTP call: {method} {base} {path}")

        with mock.patch.object(migrate, "http", side_effect=fake_http):
            migrate.migrate_config(
                src_gateway="http://src",
                dst_gateway="http://dst",
                proj_id=10,
                apply=True,
            )

        write_calls = [
            (method, path)
            for method, base, path, _ in calls
            if base == "http://dst" and method != "GET"
        ]
        self.assertEqual(
            write_calls,
            [
                ("POST", "/v1/projects"),
                ("PUT", "/v1/project/config/10"),
                ("POST", "/v1/project/config/10/versions/commit"),
                ("POST", "/v1/project/config/10/versions/dst-rev/activate"),
            ],
        )

    def test_session_tables_follow_fk_order_and_all_have_acceptance_keys(self) -> None:
        self.assertEqual(
            migrate.SESSION_TABLES,
            [
                "gateway_sessions",
                "gateway_turns",
                "gateway_runtime_iterations",
                "cc_messages",
                "gateway_feedback",
                "gateway_conversation_translate",
                "gateway_session_artifacts",
            ],
        )
        self.assertEqual(
            set(migrate.SESSION_TABLES), set(migrate.SESSION_VERIFY_KEYS)
        )
        self.assertEqual(
            migrate.SESSION_VERIFY_KEYS["cc_messages"], ("turn_id", "seq")
        )

    def test_session_apply_is_insert_only_and_heals_message_id_collision(self) -> None:
        rows = {
            "gateway_sessions": [
                {"session_id": "S1", "ds_id": 10, "proj_id": 10}
            ],
            "gateway_turns": [
                {
                    "turn_id": "T1",
                    "session_id": "S1",
                    "ds_id": 10,
                    "proj_id": 10,
                }
            ],
            "gateway_runtime_iterations": [
                {"iteration_id": "I1", "turn_id": "T1"}
            ],
            "cc_messages": [
                {
                    "message_id": 7,
                    "session_id": "S1",
                    "ds_id": 10,
                    "turn_id": "T1",
                    "iteration_id": "I1",
                    "seq": 0,
                    "role": "assistant",
                    "blocks": [{"type": "text", "text": "完成"}],
                    "usage": None,
                    "created_at_ms": 1,
                    "proj_id": 10,
                }
            ],
            "gateway_feedback": [
                {
                    "session_id": "S1",
                    "ds_id": 10,
                    "turn_id": "T1",
                    "proj_id": 10,
                }
            ],
            "gateway_conversation_translate": [
                {"session_id": "S1", "ds_id": 10, "proj_id": 10}
            ],
            "gateway_session_artifacts": [
                {"artifact_id": "A1", "proj_id": 10}
            ],
        }
        written_sql: list[tuple[str, str]] = []
        initial_message_key_query = True

        def table_from_sql(sql: str) -> str:
            for table in migrate.SESSION_TABLES:
                if f"FROM {table}" in sql:
                    return table
            self.fail(f"unknown table SQL: {sql}")

        def json_rows(url: str, sql: str) -> list[dict[str, object]]:
            table = table_from_sql(sql)
            if url == "src":
                return rows[table]
            keys = migrate.SESSION_VERIFY_KEYS[table]
            return [{key: row[key] for key in keys} for row in rows[table]]

        def columns(url: str, table: str) -> list[str]:
            result = list(rows[table][0])
            if url == "dst":
                result.append("cluster_id")
            return result

        def psql_text(url: str, sql: str) -> str:
            nonlocal initial_message_key_query
            if "setval(" in sql:
                return "100"
            if "turn_id||':'||seq::text" in sql:
                if initial_message_key_query:
                    initial_message_key_query = False
                    return ""
                return "T1:0\n"
            self.fail(f"unexpected psql_text: {sql}")

        def capture_sql(url: str, path: str) -> None:
            written_sql.append((path, Path(path).read_text(encoding="utf-8")))

        with tempfile.TemporaryDirectory() as work_dir:
            with (
                mock.patch.object(migrate, "require_psql"),
                mock.patch.object(migrate, "count_session_rows", return_value=0),
                mock.patch.object(migrate, "table_columns", side_effect=columns),
                mock.patch.object(migrate, "psql_json_rows", side_effect=json_rows),
                mock.patch.object(migrate, "psql_text", side_effect=psql_text),
                mock.patch.object(migrate, "psql_file", side_effect=capture_sql),
            ):
                migrate.migrate_sessions(
                    src_db="src",
                    dst_db="dst",
                    proj_id=10,
                    cluster_id="prod-claw-01",
                    apply=True,
                    work_dir=work_dir,
                )

        self.assertTrue(written_sql)
        self.assertTrue(
            all("ON CONFLICT DO NOTHING" in sql for _, sql in written_sql)
        )
        heal_sql = [
            sql for path, sql in written_sql if "cc_messages_heal" in path
        ]
        self.assertEqual(len(heal_sql), 1)
        self.assertNotIn("message_id", heal_sql[0].split("VALUES", 1)[0])
        self.assertIn("'prod-claw-01'", heal_sql[0])

    def test_http_verification_paginates_and_accepts_destination_superset(self) -> None:
        responses = {
            ("http://src", "/v1/projects/10/sessions?limit=100"): (
                200,
                {
                    "sessions": [
                        {
                            "sessionId": "S1",
                            "updatedAtMs": 10,
                            "turnCount": 1,
                        }
                    ],
                    "hasMore": False,
                },
            ),
            ("http://dst", "/v1/projects/10/sessions?limit=100"): (
                200,
                {
                    "sessions": [
                        {
                            "sessionId": "S1",
                            "updatedAtMs": 10,
                            "turnCount": 1,
                        },
                        {
                            "sessionId": "S-extra",
                            "updatedAtMs": 9,
                            "turnCount": 0,
                        },
                    ],
                    "hasMore": False,
                },
            ),
        }
        turn = {
            "turnId": "T1",
            "userPrompt": "迁移",
            "status": "completed",
            "reportBody": "完成",
            "createdAtMs": 1,
            "finishedAtMs": 2,
        }

        def fake_http(
            method: str,
            base: str,
            path: str,
            body: object = None,
            timeout: float = 120,
        ) -> tuple[int, object]:
            del method, body, timeout
            if path.startswith("/v1/sessions/S1/turns"):
                return 200, {"turns": [turn]}
            return responses[(base, path)]

        with mock.patch.object(migrate, "http", side_effect=fake_http):
            migrate.verify_http_sessions("http://src", "http://dst", 10)


if __name__ == "__main__":
    unittest.main()
