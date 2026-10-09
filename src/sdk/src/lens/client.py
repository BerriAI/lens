import asyncio
import json
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Final

from pydantic import TypeAdapter

from . import _native
from .config import Execution
from .errors import ConfigurationError
from .models import Report


@dataclass(frozen=True, slots=True)
class NamedEvals:
    base_url: str
    key: str = field(repr=False)

    def run(self, name: str, *, execution: Execution | None = None) -> Report:
        try:
            asyncio.get_running_loop()
        except RuntimeError:
            return asyncio.run(self.arun(name, execution=execution))
        raise ConfigurationError("Use await lens.evals.arun() inside an active event loop")

    async def arun(self, name: str, *, execution: Execution | None = None) -> Report:
        context: Final = execution or Execution.named()
        result: Final = await _native.run_named(
            name, self.base_url, self.key, json.dumps(asdict(context)), str(Path.cwd())
        )
        return TypeAdapter(Report).validate_json(result)


@dataclass(frozen=True, slots=True)
class Lens:
    base_url: str
    api_key: str = field(repr=False)

    @property
    def evals(self) -> NamedEvals:
        return NamedEvals(self.base_url, self.api_key)
