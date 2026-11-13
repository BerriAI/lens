import argparse
import asyncio
import json
import os
import time
from dataclasses import dataclass
from typing import Final
from uuid import uuid4

import httpx
from pydantic import BaseModel, ConfigDict

from lens import Case, Lens, Run


class Message(BaseModel):
    model_config = ConfigDict(frozen=True)
    content: str


class Choice(BaseModel):
    model_config = ConfigDict(frozen=True)
    message: Message


class Completion(BaseModel):
    model_config = ConfigDict(frozen=True)
    id: str
    choices: tuple[Choice, ...]


def attributes(values: dict[str, str]) -> tuple[dict[str, object], ...]:
    return tuple({"key": key, "value": {"stringValue": value}} for key, value in values.items())


@dataclass(frozen=True, slots=True)
class Agent:
    name: str

    async def run(self, case: Case, model: str, trace_id: str) -> Run:
        started: Final = time.time_ns()
        root_id: Final = uuid4().hex[:16]
        request: Final = {
            "model": model,
            "messages": [
                {"role": "system", "content": "Answer the user's question accurately in one or two sentences."},
                {"role": "user", "content": case.input},
            ],
        }
        async with httpx.AsyncClient(timeout=180) as http:
            response: Final = await http.post(
                os.environ["LITELLM_PROXY_API_BASE"].rstrip("/") + "/chat/completions",
                headers={"Authorization": f"Bearer {os.environ['LITELLM_PROXY_API_KEY']}"},
                json=request,
            )
            response.raise_for_status()
            answer: Final = Completion.model_validate_json(response.content)
            ended: Final = time.time_ns()
            common: Final = {
                "traceId": trace_id,
                "startTimeUnixNano": str(started),
                "endTimeUnixNano": str(ended),
                "status": {"code": 1},
            }
            exported: Final = await http.post(
                os.environ["LENS_TRACE_ENDPOINT"],
                headers={"Authorization": f"Bearer {os.environ['LENS_TRACE_API_KEY']}"},
                json={
                    "resourceSpans": [
                        {
                            "resource": {
                                "attributes": attributes({"service.name": self.name, "agent.name": self.name})
                            },
                            "scopeSpans": [
                                {
                                    "spans": [
                                        {
                                            **common,
                                            "spanId": root_id,
                                            "name": self.name,
                                            "attributes": attributes(
                                                {
                                                    "gen_ai.operation.name": "invoke_agent",
                                                    "gen_ai.agent.name": self.name,
                                                    "lens.optimization.request_id": case.meta.get("request_id", ""),
                                                }
                                            ),
                                        },
                                        {
                                            **common,
                                            "spanId": uuid4().hex[:16],
                                            "parentSpanId": root_id,
                                            "name": "Final answer",
                                            "attributes": attributes(
                                                {
                                                    "gen_ai.operation.name": "chat",
                                                    "gen_ai.request.model": model,
                                                    "gen_ai.response.id": answer.id,
                                                    "gen_ai.input.messages": json.dumps(request["messages"]),
                                                    "gen_ai.output.messages": json.dumps(
                                                        [
                                                            {
                                                                "role": "assistant",
                                                                "content": answer.choices[0].message.content,
                                                            }
                                                        ]
                                                    ),
                                                    "lens.replay.request": json.dumps(request),
                                                }
                                            ),
                                        },
                                    ]
                                }
                            ],
                        }
                    ]
                },
            )
            exported.raise_for_status()
        return Run(trace={"trace_id": trace_id})

    async def task(self, case: Case, model: str) -> Run:
        return await self.run(case, model, uuid4().hex)


async def main() -> None:
    parser: Final = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("live", "offline"))
    parser.add_argument("--question", default="What is 2 + 2?")
    args: Final = parser.parse_args()
    client: Final = Lens(os.environ["LENS_BASE_URL"], os.environ["LENS_API_KEY"])
    agent: Final = Agent(os.environ["LENS_OPTIMIZATION_AGENT"])
    if args.mode == "offline":
        completed: Final = await client.optimizations.arun(os.environ["LENS_OPTIMIZATION_ID"], task=agent.task)
        print(f"Submitted {completed} full-agent trials")
        return
    trace_id: Final = uuid4().hex
    assignment: Final = await client.optimizations.aassign(
        agent=agent.name,
        baseline=os.environ["LENS_OPTIMIZATION_BASELINE"],
        trace_id=trace_id,
        fail_open=False,
    )
    await agent.run(Case(trace_id, args.question), assignment.model, trace_id)
    print(json.dumps({"traceId": trace_id, "model": assignment.model, "baseline": assignment.baseline}))


if __name__ == "__main__":
    asyncio.run(main())
