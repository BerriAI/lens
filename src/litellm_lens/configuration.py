import os
from collections.abc import Mapping
from pathlib import Path
from typing import Final
from urllib.parse import urlsplit

from pydantic import Field, SecretStr, field_validator

from litellm_lens.models import Record


class Settings(Record):
    clickhouse_url: SecretStr
    database: str = Field(default="lens", pattern=r"^[A-Za-z_][A-Za-z0-9_]*$")
    admin_token: SecretStr
    service_token: SecretStr
    runtime_url: str
    public_url: str = "http://localhost:4000"
    ingestion_url: str
    ui_directory: Path | None = None

    @field_validator("admin_token", "service_token")
    @classmethod
    def strong_secret(cls, value: SecretStr) -> SecretStr:
        if len(value.get_secret_value()) < 32:
            raise ValueError("Use a secret with at least 32 characters")
        return value

    @field_validator("runtime_url", "public_url", "ingestion_url")
    @classmethod
    def service_url(cls, value: str) -> str:
        parsed: Final = urlsplit(value)
        if (
            parsed.scheme not in ("http", "https")
            or not parsed.hostname
            or parsed.username
            or parsed.password
            or parsed.query
            or parsed.fragment
            or parsed.path.rstrip("/")
        ):
            raise ValueError("Use an HTTP(S) origin without credentials, path, query or fragment")
        return value.rstrip("/")

    @field_validator("clickhouse_url")
    @classmethod
    def storage_url(cls, value: SecretStr) -> SecretStr:
        parsed: Final = urlsplit(value.get_secret_value())
        if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.query or parsed.fragment:
            raise ValueError("Use a ClickHouse HTTP(S) URL without query or fragment")
        return value

    @classmethod
    def from_env(cls, environ: Mapping[str, str] = os.environ) -> "Settings":
        return cls(
            clickhouse_url=SecretStr(environ.get("CLICKHOUSE_URL", "http://localhost:8123")),
            database=environ.get("CLICKHOUSE_DATABASE", "lens"),
            admin_token=SecretStr(environ.get("LENS_ADMIN_TOKEN", "")),
            service_token=SecretStr(environ.get("LITELLM_LENS_SERVICE_TOKEN", "")),
            runtime_url=environ.get("LITELLM_LENS_URL", "http://localhost:4318"),
            public_url=environ.get("LENS_PUBLIC_URL", "http://localhost:4000"),
            ingestion_url=environ.get("LITELLM_LENS_PUBLIC_URL", "http://localhost:4318"),
            ui_directory=Path(environ["LENS_UI_DIRECTORY"]) if environ.get("LENS_UI_DIRECTORY") else None,
        )
