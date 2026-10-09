from ._contract import CalledBefore, Judge, TaskCompleted


def task_completed() -> TaskCompleted:
    return TaskCompleted(kind="task_completed")


def called_before(first: str, then: str) -> CalledBefore:
    return CalledBefore(kind="called_before", first=first, then=then)


def judge(prompt: str, model: str = "") -> Judge:
    return Judge(kind="judge", prompt=prompt, model=model)
