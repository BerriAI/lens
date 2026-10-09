import importlib.util
import sys
from itertools import chain
from pathlib import Path
from typing import Final
from uuid import uuid4

from .errors import ConfigurationError
from .evaluation import Eval


def load_file(file: Path, name: str | None) -> tuple[Eval, ...]:
    if file.name.startswith("_"):
        return ()
    module_name: Final = f"_lens_eval_{uuid4().hex}"
    spec: Final = importlib.util.spec_from_file_location(module_name, file)
    if spec is None or spec.loader is None:
        raise ConfigurationError(f"Cannot load {file}")
    module: Final = importlib.util.module_from_spec(spec)
    original_path: Final = tuple(sys.path)
    sys.modules[module_name] = module
    try:
        sys.path[:0] = [str(file.parent.resolve()), str(Path.cwd())]
        spec.loader.exec_module(module)
    except Exception as error:
        raise ConfigurationError(f"Failed to load {file.name}: {type(error).__name__}") from error
    finally:
        sys.path[:] = original_path
        sys.modules.pop(module_name, None)
    return tuple(
        value for value in vars(module).values() if isinstance(value, Eval) and (name is None or value.name == name)
    )


def discover(path: Path, name: str | None = None) -> tuple[Eval, ...]:
    files: Final = (path,) if path.is_file() else tuple(sorted(path.rglob("*.py")))
    if not files:
        raise ConfigurationError(f"No eval files found at {path}")
    loaded: Final = chain.from_iterable(load_file(file, name) for file in files)
    evaluations: Final = tuple({id(evaluation): evaluation for evaluation in loaded}.values())
    names: Final = tuple(evaluation.name for evaluation in evaluations)
    if len(set(names)) != len(names):
        raise ConfigurationError("Each discovered eval must have a unique name")
    if not evaluations:
        raise ConfigurationError("No matching Eval instances found")
    return evaluations
