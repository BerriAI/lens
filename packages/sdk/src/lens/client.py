import asyncio
from collections.abc import Mapping
from typing import Final, TypeVar
from urllib.parse import quote

import httpx
from pydantic import BaseModel, TypeAdapter, ValidationError

from ._contract import CONTRACT_VERSION, ApiError, CaseResult, CreateEvalRun, EvalRun, ResolvedDataset
from .errors import ApiFailure, ConfigurationError, InfrastructureError
from .models import DatasetSummary, EvalCases

T = TypeVar("T", bound=BaseModel)


def error_code(response: httpx.Response) -> str:
    try:
        return ApiError.model_validate_json(response.content).code
    except ValidationError:
        return "request_failed"


class Client:
    def __init__(self, http: httpx.AsyncClient, *, attempts: int = 3, retry_delay: float = 0.25) -> None:
        if attempts < 1:
            raise ConfigurationError("HTTP attempts must be positive")
        self.http = http
        self.attempts = attempts
        self.retry_delay = retry_delay

    async def request(
        self,
        method: str,
        path: str,
        *,
        body: BaseModel | None = None,
        headers: Mapping[str, str] | None = None,
        retry: bool = True,
    ) -> httpx.Response:
        for attempt in range(self.attempts if retry else 1):
            if (response := await self._attempt(method, path, body, headers, attempt, retry)) is not None:
                return response
        raise InfrastructureError("Lens request exhausted its retry budget")

    async def _attempt(
        self,
        method: str,
        path: str,
        body: BaseModel | None,
        headers: Mapping[str, str] | None,
        attempt: int,
        retry: bool,
    ) -> httpx.Response | None:
        request_headers: Final = {"X-Lens-Contract": str(CONTRACT_VERSION), **(headers or {})}
        can_retry: Final = retry and attempt + 1 < self.attempts
        try:
            response: Final = await self.http.request(
                method,
                path,
                content=body.model_dump_json() if body else None,
                headers={"Content-Type": "application/json", **request_headers},
            )
        except httpx.TransportError as error:
            if not can_retry:
                raise InfrastructureError("Could not reach Lens within the request deadline") from error
            await asyncio.sleep(self.retry_delay * 2**attempt)
            return None
        if response.status_code in {408, 429, 500, 502, 503, 504} and can_retry:
            await asyncio.sleep(self.retry_delay * 2**attempt)
            return None
        if response.is_error:
            raise ApiFailure(response.status_code, error_code(response))
        if response.is_redirect:
            raise InfrastructureError("Lens returned an unexpected redirect")
        return response

    @staticmethod
    def decode(response: httpx.Response, model: type[T]) -> T:
        try:
            return model.model_validate_json(response.content)
        except ValidationError as error:
            raise InfrastructureError(f"Lens returned an invalid {model.__name__} response") from error

    async def resolve(self, data: str) -> ResolvedDataset:
        prefix, separator, revision_text = data.rpartition("@")
        name: Final = prefix if separator else data
        if not name or (separator and (not revision_text.isdecimal() or int(revision_text) < 1)):
            raise ConfigurationError("Use a dataset name or name@positive-revision")
        revision: Final = int(revision_text) if separator else None
        resolved: Final = await self._resolve(name, revision)
        if resolved.name != name or (revision is not None and resolved.revision != revision):
            raise InfrastructureError("Lens resolved a different dataset or revision than requested")
        return resolved

    async def _resolve(self, name: str, revision: int | None) -> ResolvedDataset:
        query: Final = f"?name={quote(name, safe='')}" + (f"&revision={revision}" if revision else "")
        try:
            response: Final = await self.request("GET", "/lens/datasets/resolve" + query)
            return self.decode(response, ResolvedDataset)
        except ApiFailure as error:
            if error.status != 404 or error.code not in {"request_failed", "dataset_not_found"}:
                raise
        listing: Final = await self.request("GET", "/lens/datasets")
        try:
            matches: Final = tuple(
                item
                for item in TypeAdapter(tuple[DatasetSummary, ...]).validate_json(listing.content)
                if item.name == name
            )
        except ValidationError as invalid:
            raise InfrastructureError("Lens returned an invalid dataset listing") from invalid
        if len(matches) != 1:
            raise ConfigurationError(f"Dataset name must identify exactly one accessible dataset: {name}")
        return ResolvedDataset(id=matches[0].id, name=name, revision=revision or matches[0].revision)

    async def cases(self, dataset: ResolvedDataset) -> EvalCases:
        response: Final = await self.request(
            "GET", f"/lens/datasets/{quote(dataset.id, safe='')}/revisions/{dataset.revision}/cases"
        )
        cases: Final = self.decode(response, EvalCases)
        if cases.dataset_id != dataset.id or cases.revision != dataset.revision:
            raise InfrastructureError("Lens returned cases from a different dataset revision")
        return cases

    async def create(self, body: CreateEvalRun, key: str) -> EvalRun:
        response: Final = await self.request("POST", "/lens/evals/runs", body=body, headers={"Idempotency-Key": key})
        return self.decode(response, EvalRun)

    async def result(self, run_id: str, case_id: str, trial: int, result: CaseResult) -> None:
        if (result.trace is None) == (result.error is None):
            raise ConfigurationError("A result requires exactly one of trace or error")
        await self.request(
            "PUT", f"/lens/evals/runs/{quote(run_id, safe='')}/results/{quote(case_id, safe='')}/{trial}", body=result
        )

    async def get(self, run_id: str, *, wait: bool = False) -> EvalRun:
        suffix: Final = "?wait=30" if wait else ""
        response: Final = await self.request("GET", f"/lens/evals/runs/{quote(run_id, safe='')}{suffix}")
        run: Final = self.decode(response, EvalRun)
        if run.id != run_id:
            raise InfrastructureError("Lens returned a different eval run")
        return run

    async def finish(self, run_id: str) -> EvalRun:
        try:
            response: Final = await self.request("POST", f"/lens/evals/runs/{quote(run_id, safe='')}/finish")
            return self.decode(response, EvalRun)
        except ApiFailure as error:
            if error.status == 409 and error.code == "run_closed":
                return await self.get(run_id)
            raise
