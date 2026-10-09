import os
from dataclasses import dataclass
from pathlib import Path
from typing import Final, Self

from pydantic import BaseModel, ConfigDict, TypeAdapter

from . import _native


class Settings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")
    project: str = ""
    evals: str = "evals/"
    base_url: str = ""

    @classmethod
    def load(cls, root: Path) -> Self:
        return cls.model_validate_json(_native.settings(str(root)))

    def endpoint(self) -> str:
        value: Final = os.environ.get("LENS_BASE_URL") or self.base_url
        return _native.endpoint(value)


@dataclass(frozen=True, slots=True)
class Execution:
    version: str
    branch: str
    pr: int | None = None
    ci_url: str = ""
    identity: str = ""

    @classmethod
    def local(cls) -> Self:
        return TypeAdapter(cls).validate_json(_native.execution(str(Path.cwd()), False))

    @classmethod
    def github(cls) -> Self:
        return TypeAdapter(cls).validate_json(_native.execution(str(Path.cwd()), True))
