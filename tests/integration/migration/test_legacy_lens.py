import os
from pathlib import Path
from typing import Final
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit
from uuid import uuid4

import psycopg
import pytest
from psycopg import sql

from litellm_lens.models import Job, Lens
from litellm_lens.state import queue_job


@pytest.mark.parametrize("populated", (False, True))
@pytest.mark.parametrize("preceding_schema", (False, True))
def test_lens_rename_preserves_saved_data_and_worker_credentials(populated: bool, preceding_schema: bool) -> None:
    migrations: Final = (
        Path(__file__).resolve().parents[3] / "migrations" / "legacy"
    )
    schema: Final = f"lens_migration_{uuid4().hex}"
    with psycopg.connect(os.environ["DATABASE_URL"]) as connection:
        try:
            connection.execute(sql.SQL("CREATE SCHEMA {}").format(sql.Identifier(schema)))
            connection.execute(sql.SQL("SET LOCAL search_path TO {}").format(sql.Identifier(schema)))
            for name in ("20260930000000_agent_engine", "20261001000000_lens_run_history"):
                connection.execute(sql.SQL((migrations / name / "migration.sql").read_text()))
            if populated:
                connection.execute(
                    "INSERT INTO \"LiteLLM_Engine\" VALUES ('lens', 7, '{\"findings\":[{\"id\":\"finding\"}]}');\n                    INSERT INTO \"LiteLLM_EngineWorker\" VALUES ('worker', 'token-hash', '{\"analysis_key_id\":\"key\"}');\n                    INSERT INTO \"LiteLLM_EngineRun\" VALUES ('batch', 'lens', '2026-01-01', '{\"cost\":1.25}')"
                )
            if preceding_schema:
                first_schema: Final = f"lens_first_{uuid4().hex}"
                connection.execute(sql.SQL("CREATE SCHEMA {}").format(sql.Identifier(first_schema)))
                connection.execute(
                    sql.SQL("SET LOCAL search_path TO {}, {}").format(
                        sql.Identifier(first_schema), sql.Identifier(schema)
                    )
                )
            connection.execute(sql.SQL((migrations / "20261001100000_rename_lens" / "migration.sql").read_text()))
            connection.execute(sql.SQL((migrations / "20261001100000_rename_lens" / "migration.sql").read_text()))
            assert connection.execute('SELECT id, version, data FROM "LiteLLM_Lens"').fetchall() == (
                [("lens", 7, {"findings": [{"id": "finding"}]})] if populated else []
            )
            assert connection.execute('SELECT id, token_hash, data FROM "LiteLLM_LensWorker"').fetchall() == (
                [("worker", "token-hash", {"analysis_key_id": "key"})] if populated else []
            )
            assert connection.execute('SELECT id, lens_id, data FROM "LiteLLM_LensRun"').fetchall() == (
                [("batch", "lens", {"cost": 1.25})] if populated else []
            )
        finally:
            connection.rollback()


@pytest.mark.parametrize("entrypoint", ("proxy", "extras-v1", "extras-v2"))
@pytest.mark.parametrize("legacy_table", ("LiteLLM_Engine", "LiteLLM_EngineRun", "LiteLLM_EngineWorker"))
def test_db_push_refuses_legacy_lens_data(monkeypatch: pytest.MonkeyPatch, entrypoint: str, legacy_table: str) -> None:
    from litellm.proxy.db.prisma_client import PrismaManager
    from litellm_proxy_extras.utils import ProxyExtrasDBManager

    database_url: Final = os.environ["DATABASE_URL"]
    schema: Final = f"lens_push_{uuid4().hex}"
    parsed: Final = urlsplit(database_url)
    scoped: Final = urlunsplit(parsed._replace(query=urlencode({**dict(parse_qsl(parsed.query)), "schema": schema})))
    with psycopg.connect(database_url, autocommit=True) as connection:
        connection.execute(sql.SQL("CREATE SCHEMA {}").format(sql.Identifier(schema)))
        try:
            connection.execute(
                sql.SQL("CREATE TABLE {} (id TEXT PRIMARY KEY, data JSONB)").format(
                    sql.Identifier(schema, legacy_table)
                )
            )
            connection.execute(
                sql.SQL("INSERT INTO {} VALUES ('saved', '{{\"keep\":true}}')").format(
                    sql.Identifier(schema, legacy_table)
                )
            )
            monkeypatch.setenv("DATABASE_URL", scoped)
            setup: Final = (
                PrismaManager.setup_database if entrypoint == "proxy" else ProxyExtrasDBManager.setup_database
            )
            with pytest.raises(RuntimeError, match="Legacy Lens tables exist"):
                setup(use_migrate=False, use_v2_resolver=entrypoint == "extras-v2")
            assert connection.execute(
                sql.SQL("SELECT id, data FROM {}").format(sql.Identifier(schema, legacy_table))
            ).fetchall() == [("saved", {"keep": True})]
        finally:
            connection.execute(sql.SQL("DROP SCHEMA {} CASCADE").format(sql.Identifier(schema)))


def test_db_push_creates_fresh_lens_tables_and_preserves_them_on_restart(monkeypatch: pytest.MonkeyPatch) -> None:
    from litellm.proxy.db.prisma_client import PrismaManager

    database_url: Final = os.environ["DATABASE_URL"]
    schema: Final = f"lens_fresh_push_{uuid4().hex}"
    parsed: Final = urlsplit(database_url)
    scoped: Final = urlunsplit(parsed._replace(query=urlencode({**dict(parse_qsl(parsed.query)), "schema": schema})))
    with psycopg.connect(database_url, autocommit=True) as connection:
        connection.execute(sql.SQL("CREATE SCHEMA {}").format(sql.Identifier(schema)))
        try:
            monkeypatch.setenv("DATABASE_URL", scoped)
            assert PrismaManager.setup_database(use_migrate=False)
            connection.execute(
                sql.SQL("INSERT INTO {} (id, data) VALUES ('saved', '{{\"keep\":true}}')").format(
                    sql.Identifier(schema, "LiteLLM_Lens")
                )
            )
            assert PrismaManager.setup_database(use_migrate=False)
            assert connection.execute(
                sql.SQL("SELECT id, data FROM {}").format(sql.Identifier(schema, "LiteLLM_Lens"))
            ).fetchall() == [("saved", {"keep": True})]
            assert (
                connection.execute(
                    sql.SQL("SELECT due_at FROM {} WHERE id='saved'").format(sql.Identifier(schema, "LiteLLM_Lens"))
                ).fetchone()[0]
                is not None
            )
            due_index: Final = connection.execute(
                "SELECT indexdef FROM pg_indexes\n                WHERE schemaname=%s AND tablename='LiteLLM_Lens' AND indexname='LiteLLM_Lens_due_at_idx'",
                (schema,),
            ).fetchone()
            assert due_index is not None
            assert "WHERE" not in due_index[0]
        finally:
            connection.execute(sql.SQL("DROP SCHEMA {} CASCADE").format(sql.Identifier(schema)))


def test_review_migration_preserves_existing_lens_history_credentials_and_spend() -> None:
    from tests.unit.litellm_lens.test_state import NOW, lens, worker

    migrations: Final = (
        Path(__file__).resolve().parents[3] / "migrations" / "legacy"
    )
    schema: Final = f"lens_reviews_{uuid4().hex}"
    legacy: Final = lens().model_dump_json(exclude={"criteria_updated_at", "reservations"})
    job: Final = (
        queue_job(lens(), NOW, "archived")
        .jobs[0]
        .model_dump_json(exclude={"review_versions": True, "coverage": {"reused"}})
    )
    with psycopg.connect(os.environ["DATABASE_URL"]) as connection:
        try:
            connection.execute(sql.SQL("CREATE SCHEMA {}").format(sql.Identifier(schema)))
            connection.execute(sql.SQL("SET LOCAL search_path TO {}").format(sql.Identifier(schema)))
            for name in (
                "20260930000000_agent_engine",
                "20261001000000_lens_run_history",
                "20261001100000_rename_lens",
            ):
                connection.execute(sql.SQL((migrations / name / "migration.sql").read_text()))
            connection.execute('INSERT INTO "LiteLLM_Lens" VALUES (%s, 0, %s)', ("lens", legacy))
            connection.execute('INSERT INTO "LiteLLM_LensRun" VALUES (%s, %s, %s, %s)', ("archived", "lens", NOW, job))
            connection.execute(
                'INSERT INTO "LiteLLM_LensWorker" VALUES (%s, %s, %s)',
                ("worker", "existing-token", worker().model_dump_json()),
            )
            before: Final = tuple(
                connection.execute(sql.SQL("SELECT * FROM {}").format(sql.Identifier(table))).fetchall()
                for table in ("LiteLLM_Lens", "LiteLLM_LensRun", "LiteLLM_LensWorker")
            )
            connection.execute(
                sql.SQL((migrations / "20261006000000_lens_review_checkpoints" / "migration.sql").read_text())
            )
            connection.execute(
                sql.SQL((migrations / "20261006000000_lens_review_checkpoints" / "migration.sql").read_text())
            )
            after: Final = tuple(
                connection.execute(sql.SQL("SELECT * FROM {}").format(sql.Identifier(table))).fetchall()
                for table in ("LiteLLM_Lens", "LiteLLM_LensRun", "LiteLLM_LensWorker")
            )
            assert after == before
            assert Lens.model_validate(after[0][0][-1]) == lens()
            assert Job.model_validate(after[1][0][-1]) == queue_job(lens(), NOW, "archived").jobs[0]
            assert connection.execute('SELECT count(*) FROM "LiteLLM_LensReview"').fetchone() == (0,)
        finally:
            connection.rollback()
