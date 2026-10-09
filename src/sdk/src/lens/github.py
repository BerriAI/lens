import sys

from .cli import main as cli_main


def main() -> None:
    raise SystemExit(cli_main(["report", *sys.argv[1:]]))


if __name__ == "__main__":
    main()
