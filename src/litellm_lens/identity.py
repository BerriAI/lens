from dataclasses import dataclass
from enum import StrEnum
from typing import TypeAlias

from pydantic import ConfigDict

from litellm_lens.models import Record


class Role(StrEnum):
    PROXY_ADMIN = "proxy_admin"
    PROXY_ADMIN_VIEW_ONLY = "proxy_admin_viewer"
    ORG_ADMIN = "org_admin"
    INTERNAL_USER = "internal_user"
    INTERNAL_USER_VIEW_ONLY = "internal_user_viewer"
    TEAM = "team"
    CUSTOMER = "customer"


class Identity(Record):
    model_config = ConfigDict(frozen=True, extra="forbid")
    user_role: Role = Role.INTERNAL_USER
    user_id: str | None = None
    team_id: str | None = None
    org_id: str | None = None
    token: str | None = None
    models: tuple[str, ...] = ()
    log_team_ids: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class AllRows:
    pass


@dataclass(frozen=True, slots=True)
class OwnedRows:
    user_id: str | None
    team_ids: tuple[str, ...] = ()


ReadScope: TypeAlias = AllRows | OwnedRows


class ModelRequestError(Exception):
    def __init__(
        self, message: str, code: int, openai_code: str | None = None, headers: dict[str, str] | None = None
    ) -> None:
        super().__init__(message)
        self.message = message
        self.code = str(code)
        self.openai_code = openai_code
        self.headers = headers or {}


class KeyNotFoundError(Exception):
    pass
