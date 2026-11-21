from agent import litellm_agent, openai_agents_agent
from lens import AgentOutput, Target, TaskInput, Verdict

SONNET = "claude-sonnet-5-5"
MINI = "gpt-5.4-mini"

targets = [
    Target(harness="litellm", model=SONNET, run=litellm_agent),
    Target(harness="litellm", model=MINI, run=litellm_agent),
    Target(harness="openai-agents", model=MINI, run=openai_agents_agent),
]


def check(task: TaskInput, output: AgentOutput) -> Verdict:
    answer = output.text.strip().strip(".").casefold()
    expected = (task.expected or "").casefold()
    if answer == expected:
        return Verdict(True)
    return Verdict(False, f"expected {expected}, got {answer or 'nothing'}")
