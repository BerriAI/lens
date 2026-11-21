import json
from collections.abc import Mapping
from types import MappingProxyType
from typing import Final

import litellm
from agents import Agent, ModelSettings, RunContextWrapper, Runner, function_tool
from agents.items import ToolCallItem
from litellm import Choices, ModelResponse
from litellm.types.utils import ChatCompletionMessageToolCall

from lens import AgentOutput, TaskInput, ToolCall

INSTRUCTIONS: Final = (
    "You triage customer support tickets. Classify each ticket as exactly one of: billing, technical, sales. "
    "When a ticket is referenced only by id, call lookup_ticket to read it first. "
    "Reply with only the category word."
)

TICKETS: Final[Mapping[str, str]] = MappingProxyType(
    {
        "T-1001": "I was charged twice for my March invoice and need a refund.",
        "T-1002": "The desktop app crashes every time I open the settings page.",
        "T-1003": "We are a team of 40 and want a quote for the enterprise plan.",
        "T-1004": "My card was declined but the money still left my account.",
        "T-1005": "SSO login fails with error 502 since this morning.",
        "T-1006": "Can we get volume pricing if we add another 25 seats?",
        "T-1007": "The export to CSV button does nothing in Firefox.",
        "T-1008": "Please update the VAT number printed on our invoices.",
    }
)

LOOKUP_TOOL: Final = {
    "type": "function",
    "function": {
        "name": "lookup_ticket",
        "description": "Read the customer's message for a support ticket id",
        "parameters": {
            "type": "object",
            "properties": {"ticket_id": {"type": "string"}},
            "required": ["ticket_id"],
        },
    },
}


def lookup(ticket_id: str) -> str:
    return TICKETS.get(ticket_id.strip().upper(), f"No ticket named {ticket_id}")


def _tool_call(name: str, raw: str) -> ToolCall:
    try:
        arguments: Final = json.loads(raw or "{}")
    except json.JSONDecodeError:
        return ToolCall(name=name, arguments={"raw": raw}, error=True)
    known: Final = name == "lookup_ticket" and str(arguments.get("ticket_id", "")).strip().upper() in TICKETS
    return ToolCall(name=name, arguments=arguments, error=not known)


async def litellm_agent(task: TaskInput) -> AgentOutput:
    messages: Final[list[dict[str, object]]] = [  # mutable-ok: litellm takes a growing message list
        {"role": "system", "content": INSTRUCTIONS},
        {"role": "user", "content": task.prompt},
    ]
    calls: Final[list[ToolCall]] = []  # mutable-ok: tool calls accumulate across turns
    cost = 0.0
    for _ in range(4):
        response = await litellm.acompletion(model=task.model, messages=messages, tools=[LOOKUP_TOOL])
        if not isinstance(response, ModelResponse) or not isinstance(response.choices[0], Choices):
            raise TypeError("Expected a non-streaming chat completion")
        cost += litellm.completion_cost(completion_response=response) or 0.0
        message = response.choices[0].message
        requested = [call for call in message.tool_calls or [] if isinstance(call, ChatCompletionMessageToolCall)]
        if not requested:
            return AgentOutput(text=(message.content or "").strip(), cost=cost, tool_calls=tuple(calls))
        messages.append(message.model_dump(exclude_none=True))
        for call in requested:
            recorded = _tool_call(call.function.name or "unknown", call.function.arguments)
            calls.append(recorded)
            messages.append(
                {
                    "role": "tool",
                    "tool_call_id": call.id,
                    "content": lookup(str(recorded.arguments.get("ticket_id", ""))),
                }
            )
    return AgentOutput(text="", cost=cost, tool_calls=tuple(calls))


@function_tool
def lookup_ticket(context: RunContextWrapper[None], ticket_id: str) -> str:
    """Read the customer's message for a support ticket id."""
    return lookup(ticket_id)


async def openai_agents_agent(task: TaskInput) -> AgentOutput:
    agent: Final = Agent(
        name="SupportTriage",
        instructions=INSTRUCTIONS,
        model=task.model,
        model_settings=ModelSettings(),
        tools=[lookup_ticket],
    )
    result: Final = await Runner.run(agent, task.prompt, max_turns=4)
    usage: Final = result.context_wrapper.usage
    prompt_cost, completion_cost = litellm.cost_per_token(
        model=task.model, prompt_tokens=usage.input_tokens, completion_tokens=usage.output_tokens
    )
    calls: Final = tuple(
        _tool_call(getattr(item.raw_item, "name", "unknown"), getattr(item.raw_item, "arguments", "{}"))
        for item in result.new_items
        if isinstance(item, ToolCallItem)
    )
    return AgentOutput(text=str(result.final_output).strip(), cost=prompt_cost + completion_cost, tool_calls=calls)
