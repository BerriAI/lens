import importlib.util
import json
from pathlib import Path

import pytest
from pydantic import ValidationError

from lens import Gate, scorers
from lens._contract import CaseResult, CreateEvalRun, EvalRun

SDK = Path(__file__).resolve().parents[1]
SHARED = SDK.parents[1] / "src/worker/crates/contract/fixtures/lens_eval"
FIXTURES = SHARED if SHARED.exists() else SDK / "tests/fixtures/lens_eval"


def appendix():
    spec = importlib.util.spec_from_file_location("appendix", SDK / "tests/fixtures/eval_contract.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.mark.parametrize(
    "name,model",
    [
        ("create_run", CreateEvalRun),
        ("case_result_trace", CaseResult),
        ("case_result_error", CaseResult),
        ("eval_run_done", EvalRun),
        ("eval_run_no_baseline", EvalRun),
    ],
)
def test_golden_wire_payloads_match_appendix(name, model):
    payload = (FIXTURES / f"{name}.json").read_text()
    original = getattr(appendix(), model.__name__).model_validate_json(payload)
    generated = model.model_validate_json(payload)
    assert generated.model_dump(mode="json", exclude={"agent_io", "output", "baseline_run_id"}) == original.model_dump(
        mode="json"
    )
    assert model.model_validate_json(generated.model_dump_json()) == generated


@pytest.mark.parametrize(
    "value",
    [
        {"trials": 0},
        {"revision": 0},
        {"scorers": []},
        {"unknown": True},
        {"timeout_per_trial_ms": 0},
        {"timeout_per_trial_ms": 2**64},
    ],
)
def test_generated_models_reject_invalid_requests(value):
    payload = json.loads((FIXTURES / "create_run.json").read_text())
    for model in (CreateEvalRun, appendix().CreateEvalRun):
        with pytest.raises(ValidationError):
            model.model_validate({**payload, **value})


def test_generated_defaults_and_nulls_keep_public_numeric_api():
    gate = Gate()
    assert gate.regressions == 0 and gate.critical == 0
    assert Gate(pass_rate=0.75).pass_rate == 0.75
    assert Gate(regressions=None).regressions is None
    with pytest.raises(ValidationError):
        Gate(pass_rate=1.01)
    assert scorers.called_before("test", "commit").first == "test"
