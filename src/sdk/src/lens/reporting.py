from pydantic import TypeAdapter

from . import _native
from .models import Report


def terminal(report: Report) -> str:
    return _native.render(TypeAdapter(Report).dump_json(report).decode(), "terminal")


def markdown(report: Report) -> str:
    return _native.render(TypeAdapter(Report).dump_json(report).decode(), "markdown")


def conclusion(report: Report) -> str:
    return _native.render(TypeAdapter(Report).dump_json(report).decode(), "conclusion")
