from lens import Case, Eval, Gate, Run, scorers


async def task(case: Case) -> Run:
    return Run(trace={"session.id": f"pass-{case.id}"}, cost_usd=0.01)


evaluation = Eval(
    "demo", task=task, data="demo@1", scores=[scorers.task_completed()], trials=3, gate=Gate(pass_rate=0.9)
)
