import hashlib
from importlib.resources import files
from typing import Final, LiteralString, cast

from pydantic import TypeAdapter

from litellm_lens.database import Database
from litellm_lens.models import Record


class AppliedMigration(Record):
    name: str
    checksum: str


async def migrate(database: Database) -> None:
    async with database.transaction() as transaction:
        await transaction.query_raw("SELECT pg_advisory_xact_lock(746362184947742)")
        await transaction.execute_raw(
            'CREATE TABLE IF NOT EXISTS "LensSchemaMigration" '
            "(name TEXT PRIMARY KEY, checksum TEXT NOT NULL, applied_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP)"
        )
        applied: Final = TypeAdapter(tuple[AppliedMigration, ...]).validate_python(
            await transaction.query_raw('SELECT name, checksum FROM "LensSchemaMigration" ORDER BY name')
        )
        available: Final = tuple(
            sorted(
                (path for path in files("litellm_lens.migrations").iterdir() if path.name.endswith(".sql")),
                key=lambda path: path.name,
            )
        )
        if any(record.name not in {path.name for path in available} for record in applied):
            raise RuntimeError("The database has newer Lens migrations; use a compatible Lens release")
        for path in available:
            content: Final = path.read_bytes()
            checksum: Final = hashlib.sha256(content).hexdigest()
            previous: Final = next((record for record in applied if record.name == path.name), None)
            if previous is not None:
                if previous.checksum != checksum:
                    raise RuntimeError(f"Applied Lens migration changed: {path.name}")
                continue
            await transaction.execute_raw(cast(LiteralString, content.decode()))
            await transaction.execute_raw(
                'INSERT INTO "LensSchemaMigration" (name, checksum) VALUES ($1, $2)', path.name, checksum
            )
