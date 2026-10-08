from typing import Final

import pytest
from pydantic import SecretStr, ValidationError

from litellm_lens.configuration import Settings


@pytest.mark.parametrize(
    "url",
    (
        "file:///tmp/lens",
        "http://name:secret@localhost:4318",
        "http://localhost:4318/internal",
        "http://localhost:4318?token=secret",
        "http://localhost:4318#fragment",
    ),
)
def test_service_urls_reject_credentials_and_ambiguous_destinations(url: str) -> None:
    with pytest.raises(ValidationError):
        Settings(
            clickhouse_url=SecretStr("http://localhost:8123"),
            admin_token=SecretStr("a" * 32),
            service_token=SecretStr("s" * 32),
            runtime_url=url,
            ingestion_url="http://localhost:4318",
        )


def test_missing_credentials_fail_before_starting_a_public_api() -> None:
    with pytest.raises(ValidationError, match="at least 32 characters"):
        Settings.from_env({})


def test_database_credentials_are_not_rendered_in_settings_or_validation_errors() -> None:
    config: Final = Settings.from_env(
        {
            "LENS_ADMIN_TOKEN": "private-admin-token" * 3,
            "LITELLM_LENS_SERVICE_TOKEN": "private-service-token" * 3,
            "CLICKHOUSE_URL": "http://lens:private-database-password@localhost:8123",
        }
    )
    assert "private" not in repr(config)
    with pytest.raises(ValidationError) as failure:
        Settings.from_env(
            {
                "LENS_ADMIN_TOKEN": "private",
                "CLICKHOUSE_URL": "file:///private-database-password",
            }
        )
    assert "private" not in str(failure.value)
