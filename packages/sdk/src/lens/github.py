import argparse
import asyncio
import json
import os
from itertools import count
from pathlib import Path
from typing import Final
from urllib.parse import quote

import httpx
from pydantic import BaseModel, ConfigDict, TypeAdapter

from ._contract import EvalRun
from .client import Client
from .config import Settings
from .errors import ConfigurationError, InfrastructureError
from .models import Report
from .reporting import conclusion, markdown


class Comment(BaseModel):
    model_config = ConfigDict(frozen=True, extra="ignore")
    id: int
    body: str | None = None
    user: dict[str, str | int | bool | None] = {}


class GitHub:
    def __init__(self, http: httpx.AsyncClient, repository: str) -> None:
        if len(repository.split("/")) != 2 or any(not part for part in repository.split("/")):
            raise ConfigurationError("Expected GitHub repository owner/name")
        self.http = http
        self.repository = "/".join(quote(part, safe="") for part in repository.split("/"))

    async def request(self, method: str, path: str, body: dict[str, object] | None = None) -> httpx.Response:
        response: Final = await self.http.request(method, f"/repos/{self.repository}{path}", json=body)
        if response.is_error or response.is_redirect:
            raise InfrastructureError(f"GitHub report request failed with HTTP {response.status_code}")
        return response

    async def publish(self, report: Report, sha: str) -> None:
        body: Final = markdown(report)
        if report.run.pr is not None:
            for page in count(1):
                if await self._comment_page(report.run.pr, report.run.eval, body, page):
                    break
        await self.request(
            "POST",
            "/check-runs",
            {
                "name": f"Lens / {report.run.eval}",
                "head_sha": sha,
                "status": "completed",
                "conclusion": conclusion(report),
                "details_url": report.url,
                "output": {"title": f"Lens / {report.run.eval}", "summary": body[:65000]},
            },
        )

    async def _comment_page(self, pr: int, name: str, body: str, page: int) -> bool:
        marker: Final = f"<!-- lens:{name} -->"
        response: Final = await self.request("GET", f"/issues/{pr}/comments?per_page=100&page={page}")
        comments: Final = TypeAdapter(tuple[Comment, ...]).validate_json(response.content)
        existing: Final = next(
            (
                comment
                for comment in comments
                if (comment.body or "").startswith(marker) and comment.user.get("login") == "github-actions[bot]"
            ),
            None,
        )
        if existing is not None:
            await self.request("PATCH", f"/issues/comments/{existing.id}", {"body": body})
            return True
        if len(comments) < 100:
            await self.request("POST", f"/issues/{pr}/comments", {"body": body})
            return True
        return False


async def publish_run(run: EvalRun, reporter: GitHub, lens: Client, sha: str) -> None:
    report: Final = Report(run)
    baseline: Final = await lens.get(report.summary.baseline_run_id) if report.summary.baseline_run_id else None
    await reporter.publish(Report(run, baseline), sha)


async def publish_file(path: Path) -> None:
    token: Final = os.environ.get("GITHUB_TOKEN", "")
    key: Final = os.environ.get("LENS_API_KEY", "")
    repository: Final = os.environ.get("GITHUB_REPOSITORY", "")
    sha: Final = os.environ.get("GITHUB_SHA", "")
    if not token or not key or not sha:
        raise ConfigurationError("GitHub reporting requires GITHUB_TOKEN, GITHUB_SHA and LENS_API_KEY")
    payload: Final = json.loads(path.read_text())
    runs: Final = TypeAdapter(tuple[EvalRun, ...]).validate_python(payload["runs"])
    if not runs:
        raise InfrastructureError("No completed Lens runs to report")
    settings: Final = Settings.load(Path.cwd())
    async with httpx.AsyncClient(
        base_url=settings.endpoint(), headers={"Authorization": f"Bearer {key}"}, timeout=40
    ) as lens:
        async with httpx.AsyncClient(
            base_url=os.environ.get("GITHUB_API_URL", "https://api.github.com"),
            headers={
                "Authorization": f"Bearer {token}",
                "Accept": "application/vnd.github+json",
                "X-GitHub-Api-Version": "2022-11-28",
            },
            timeout=30,
        ) as github:
            reporter: Final = GitHub(github, repository)
            for run in runs:
                await publish_run(run, reporter, Client(lens), sha)
    output: Final = os.environ.get("GITHUB_OUTPUT")
    if output:
        with Path(output).open("a") as stream:
            stream.write(f"passed={str(all(Report(run).summary.gate.passed for run in runs)).lower()}\n")
            stream.write(f"run-urls={json.dumps([run.url for run in runs])}\n")


def main() -> None:
    parser: Final = argparse.ArgumentParser()
    parser.add_argument("report", type=Path)
    args: Final = parser.parse_args()
    asyncio.run(publish_file(args.report))


if __name__ == "__main__":
    main()
