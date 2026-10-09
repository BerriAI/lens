import json
import os
import time
from typing import Final
from uuid import uuid4

import httpx
from pydantic import BaseModel, ConfigDict

from lens import Case, Eval, Gate, Run, scorers


class Message(BaseModel):
    model_config = ConfigDict(frozen=True, extra="ignore")
    content: str


class Choice(BaseModel):
    model_config = ConfigDict(frozen=True, extra="ignore")
    message: Message


class Completion(BaseModel):
    model_config = ConfigDict(frozen=True, extra="ignore")
    id: str
    choices: tuple[Choice, ...]


async def task(case: Case) -> Run:
    session: Final = f"pass-sdk-smoke-{uuid4().hex}"
    trace_id: Final = uuid4().hex
    started: Final = time.time_ns()
    model: Final = os.environ["LENS_SMOKE_MODEL"]
    async with httpx.AsyncClient(timeout=120) as http:
        response: Final = await http.post(
            os.environ["GATEWAY_BASE_URL"].rstrip("/") + "/v1/chat/completions",
            headers={"Authorization": f"Bearer {os.environ['GATEWAY_API_KEY']}"},
            json={
                "model": model,
                "messages": [
                    {"role": "system", "content": "Write a brief response to this support request. Do not use tools."},
                    {"role": "user", "content": case.input},
                ],
                "max_completion_tokens": 512,
            },
        )
        response.raise_for_status()
        completion: Final = Completion.model_validate_json(response.content)
        if not completion.choices or not completion.choices[0].message.content.strip():
            raise ValueError("The gateway returned no response text")
        attributes: Final = {
            "session.id": session,
            "agent.name": "lens-sdk-smoke",
            "agent.version": os.environ["AGENT_BUILD_SHA"],
            "deployment.environment": "lens-eval",
            "gen_ai.request.model": model,
            "gen_ai.response.id": completion.id,
            "gen_ai.input.messages": json.dumps([{"role": "user", "content": case.input}]),
            "gen_ai.output.messages": json.dumps([completion.choices[0].message.model_dump()]),
        }
        exported: Final = await http.post(
            os.environ["LENS_TRACE_ENDPOINT"],
            headers={"Authorization": f"Bearer {os.environ['LENS_TRACE_API_KEY']}"},
            json={
                "resourceSpans": [
                    {
                        "resource": {
                            "attributes": [{"key": "service.name", "value": {"stringValue": "lens-sdk-smoke"}}]
                        },
                        "scopeSpans": [
                            {
                                "scope": {"name": "lens-sdk-smoke"},
                                "spans": [
                                    {
                                        "traceId": trace_id,
                                        "spanId": uuid4().hex[:16],
                                        "name": "lens-sdk-live-trial",
                                        "kind": 1,
                                        "startTimeUnixNano": str(started),
                                        "endTimeUnixNano": str(time.time_ns()),
                                        "status": {"code": 1},
                                        "attributes": [
                                            {"key": key, "value": {"stringValue": value}}
                                            for key, value in attributes.items()
                                        ],
                                    }
                                ],
                            }
                        ],
                    }
                ],
            },
        )
        exported.raise_for_status()
    cost_header: Final = response.headers.get("x-litellm-response-cost")
    cost: Final = float(cost_header) if cost_header else None
    print(
        json.dumps(
            {
                "case_id": case.id,
                "trace_id": trace_id,
                "session_id": session,
                "model": model,
                "inference_status": response.status_code,
                "ingest_status": exported.status_code,
                "cost_usd": cost,
                "response_characters": len(completion.choices[0].message.content),
            }
        )
    )
    return Run(trace={"session.id": session}, cost_usd=cost)


evaluation: Final = Eval(
    "sdk-live-smoke",
    task=task,
    data=os.environ.get("LENS_SMOKE_DATASET", "demo@1"),
    scores=[scorers.task_completed()],
    trials=3,
    gate=Gate(pass_rate=1),
)
