import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Final

from pydantic import JsonValue, RootModel


class Schema(RootModel[JsonValue]):
    model_config = {"frozen": True}


SDK: Final = Path(__file__).resolve().parents[1]
REPOSITORY: Final = SDK.parents[1]


UNSIGNED_MAXIMUM: Final = {"uint8": 2**8 - 1, "uint16": 2**16 - 1, "uint32": 2**32 - 1, "uint64": 2**64 - 1}


def normalize(value: JsonValue) -> JsonValue:
    if isinstance(value, list):
        return [normalize(item) for item in value]
    if not isinstance(value, dict):
        return value
    if value.get("additionalProperties") is True:
        return normalize({**value, "additionalProperties": {"customTypePath": "pydantic.JsonValue"}})
    width: Final = value.get("format")
    if isinstance(width, str) and width in UNSIGNED_MAXIMUM:
        return normalize(
            {"maximum": UNSIGNED_MAXIMUM[width], **{key: item for key, item in value.items() if key != "format"}}
        )
    branches: Final = value.get("anyOf")
    if not isinstance(branches, list):
        return {key: normalize(item) for key, item in value.items()}
    nonnull: Final = tuple(branch for branch in branches if isinstance(branch, dict) and branch.get("type") != "null")
    if (
        len(branches) == 2
        and len(nonnull) == 1
        and nonnull[0].get("type") in ("number", "integer", "string", "boolean")
    ):
        return normalize(
            {
                **{key: item for key, item in value.items() if key != "anyOf"},
                **nonnull[0],
                "type": [nonnull[0]["type"], "null"],
            }
        )
    return {key: normalize(item) for key, item in value.items()}


def main() -> int:
    parser: Final = argparse.ArgumentParser()
    parser.add_argument("--schema", type=Path, default=REPOSITORY / "schema/lens.v1.json")
    parser.add_argument("--output", type=Path, default=SDK / "src/lens/_generated.py")
    parser.add_argument("--check", action="store_true")
    args: Final = parser.parse_args()
    if not args.schema.is_file():
        parser.error(f"Rust-exported schema has not landed: {args.schema}")
    with tempfile.TemporaryDirectory() as directory:
        output: Final = Path(directory) / "_generated.py"
        schema: Final = Path(directory) / "lens.v1.json"
        schema.write_text(json.dumps(normalize(Schema.model_validate_json(args.schema.read_text()).root)))
        subprocess.run(
            [
                sys.executable,
                "-m",
                "datamodel_code_generator",
                "--input",
                str(schema),
                "--input-file-type",
                "jsonschema",
                "--output",
                str(output),
                "--output-model-type",
                "pydantic_v2.BaseModel",
                "--target-python-version",
                "3.11",
                "--use-union-operator",
                "--use-standard-collections",
                "--enum-field-as-literal",
                "all",
                "--enable-faux-immutability",
                "--disable-timestamp",
                "--strict-nullable",
                "--use-generic-container-types",
                "--field-constraints",
                "--use-default-kwarg",
                "--formatters",
                "ruff-check",
                "ruff-format",
                "--collapse-root-models",
                "--use-title-as-name",
            ],
            check=True,
        )
        subprocess.run(
            [sys.executable, "-m", "ruff", "format", "--config", str(SDK / "pyproject.toml"), str(output)],
            check=True,
            capture_output=True,
        )
        content: Final = output.read_bytes()
        if args.check:
            if not args.output.exists() or args.output.read_bytes() != content:
                print(f"Generated Pydantic models drift from {args.schema}", file=sys.stderr)
                return 1
        else:
            args.output.write_bytes(content)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
