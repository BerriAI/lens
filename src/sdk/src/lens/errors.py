class LensError(Exception):
    pass


class ConfigurationError(LensError):
    pass


class InfrastructureError(LensError):
    pass


class ApiFailure(InfrastructureError):
    def __init__(self, status: int, code: str, hint: str = "") -> None:
        self.status = status
        self.code = code
        super().__init__(f"Lens HTTP {status}: {code}" + (f". {hint}" if hint else ""))


class GateFailed(AssertionError):
    pass
