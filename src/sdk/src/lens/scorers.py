from ._contract import CalledBefore, Judge, TaskCompleted


def task_completed() -> TaskCompleted:
    return TaskCompleted()


def called_before(first: str, then: str) -> CalledBefore:
    return CalledBefore(first=first, then=then)


def judge(prompt: str, model: str = "") -> Judge:
    return Judge(prompt=prompt, model=model)
