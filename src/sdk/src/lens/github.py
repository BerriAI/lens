import json
import sys

from . import _native


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("Usage: python -m lens.github REPORT.json")
    _native.control(json.dumps({"command": "report", "file": sys.argv[1]}))


if __name__ == "__main__":
    main()
