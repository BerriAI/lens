import os
import subprocess
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Final, Self
from urllib.parse import urlsplit
from uuid import uuid4

from pydantic import BaseModel, ConfigDict, ValidationError

from .errors import ConfigurationError


class PullRequest(BaseModel):
    model_config = ConfigDict(frozen=True, extra="ignore")
    number: int


class Event(BaseModel):
    model_config = ConfigDict(frozen=True, extra="ignore")
    pull_request: PullRequest | None = None


class Settings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")
    project: str = ""
    evals: str = "evals/"
    base_url: str = ""

    @classmethod
    def load(cls, root: Path) -> Self:
        path: Final = root / "pyproject.toml"
        try:
            config: Final = tomllib.loads(path.read_text()) if path.exists() else {}
            return cls.model_validate(config.get("tool", {}).get("lens", {}))
        except (OSError, ValueError, TypeError, AttributeError, ValidationError) as error:
            raise ConfigurationError("Invalid [tool.lens] settings") from error

    def endpoint(self) -> str:
        value: Final = os.environ.get("LENS_BASE_URL") or self.base_url
        parts: Final = urlsplit(value)
        if parts.scheme not in {"https", "http"} or not parts.hostname or parts.username or parts.password:
            raise ConfigurationError("Set LENS_BASE_URL or [tool.lens].base_url to the Lens server URL")
        if parts.query or parts.fragment:
            raise ConfigurationError("Lens base URL cannot contain a query or fragment")
        if parts.scheme == "http" and parts.hostname not in {"localhost", "127.0.0.1", "::1"}:
            raise ConfigurationError("Remote Lens servers require HTTPS")
        return value.rstrip("/")


def git_value(*arguments: str) -> str:
    result: Final = subprocess.run(["git", *arguments], capture_output=True, text=True, check=False)
    if result.returncode:
        raise ConfigurationError("Run inside a Git checkout or set LENS_VERSION and LENS_BRANCH")
    return result.stdout.strip()


@dataclass(frozen=True, slots=True)
class Execution:
    version: str
    branch: str
    pr: int | None = None
    ci_url: str = ""
    identity: str = ""

    @classmethod
    def local(cls) -> Self:
        return cls(
            os.environ.get("LENS_VERSION") or git_value("rev-parse", "HEAD"),
            os.environ.get("LENS_BRANCH") or git_value("rev-parse", "--abbrev-ref", "HEAD"),
            identity=os.environ.get("LENS_EXECUTION_ID") or uuid4().hex,
        )

    @classmethod
    def github(cls) -> Self:
        try:
            event: Final = Event.model_validate_json(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
            pull: Final = event.pull_request
            version: Final = os.environ["GITHUB_SHA"]
            branch: Final = os.environ.get("GITHUB_HEAD_REF") or os.environ["GITHUB_REF_NAME"]
            pr: Final = pull.number if pull else None
            run_id: Final = os.environ["GITHUB_RUN_ID"]
            attempt: Final = os.environ.get("GITHUB_RUN_ATTEMPT", "1")
            server: Final = os.environ.get("GITHUB_SERVER_URL", "https://github.com").rstrip("/")
            repository: Final = os.environ["GITHUB_REPOSITORY"]
        except (KeyError, ValueError, OSError, TypeError) as error:
            raise ConfigurationError("Incomplete GitHub Actions execution context") from error
        return cls(version, branch, pr, f"{server}/{repository}/actions/runs/{run_id}", f"{run_id}:{attempt}")
