import json
import threading
from collections.abc import Iterator
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from queue import Queue
from typing import Final
from uuid import uuid4

import pytest

from lens import Case, Lens, Run
from lens.errors import InfrastructureError


@pytest.fixture
def optimization_server() -> Iterator[tuple[str, Queue[tuple[int, bytes]], Queue[tuple[str, bytes]]]]:
    replies: Final[Queue[tuple[int, bytes]]] = Queue()
    requests: Final[Queue[tuple[str, bytes]]] = Queue()

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:
            body: Final = self.rfile.read(int(self.headers["Content-Length"]))
            requests.put((self.path, body))
            code, reply = replies.get(timeout=2)
            self.send_response(code)
            self.send_header("Content-Length", str(len(reply)))
            self.end_headers()
            self.wfile.write(reply)

        def log_message(self, format: str, *args: object) -> None:
            return

    server: Final = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread: Final = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", replies, requests
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


def test_assignment_uses_trace_identity_and_fails_open_without_leaking_credentials(optimization_server):
    endpoint, replies, requests = optimization_server
    replies.put((200, b'{"optimizationId":"experiment","traceId":"trace","model":"candidate","baseline":false}'))
    client: Final = Lens(endpoint, "test-only-private-key")
    assignment: Final = client.optimizations.assign(agent="agent", baseline="baseline", trace_id="trace")
    assert assignment.model == "candidate" and assignment.optimization_id == "experiment"
    route, body = requests.get_nowait()
    assert route == "/lens/optimizations/assign"
    assert json.loads(body) == {"agent": "agent", "baseline": "baseline", "traceId": "trace"}
    replies.put((503, b"private server diagnostic"))
    assert client.optimizations.assign(agent="agent", baseline="baseline", trace_id="next").model == "baseline"
    assert "test-only-private-key" not in repr(client.optimizations)
    replies.put((403, b"private server diagnostic"))
    with pytest.raises(InfrastructureError, match="Lens rejected") as error:
        client.optimizations.assign(agent="agent", baseline="baseline", trace_id="next", fail_open=False)
    assert "private server diagnostic" not in str(error.value)


async def test_full_agent_runner_handles_more_trials_than_python_recursion_limit(optimization_server):
    endpoint, replies, requests = optimization_server
    for index in range(1050):
        replies.put(
            (
                200,
                json.dumps(
                    {
                        "id": str(index),
                        "lease": "test-lease",
                        "model": "candidate",
                        "input": "2+2",
                        "expected": "4",
                        "requestId": f"trial-{index}",
                    }
                ).encode(),
            )
        )
        replies.put((200, b'{"accepted":true}'))
    replies.put((200, b"null"))

    async def task(case: Case, model: str) -> Run:
        assert model == "candidate"
        assert case.meta["request_id"] == f"trial-{case.id}"
        return Run(trace={"trace_id": f"trace-{case.id}"})

    completed: Final = await Lens(endpoint, "test-only").optimizations.arun(str(uuid4()), task=task)
    assert completed == 1050
    assert requests.qsize() == 2101


async def test_original_agent_failure_survives_error_submission_failure(optimization_server):
    endpoint, replies, _ = optimization_server
    replies.put(
        (200, b'{"id":"case","lease":"test-lease","model":"candidate","input":"q","expected":"a","requestId":"trial"}')
    )
    replies.put((503, b"unavailable"))

    async def task(case: Case, model: str) -> Run:
        raise ValueError("agent failed locally")

    with pytest.raises(ValueError, match="agent failed locally"):
        await Lens(endpoint, "test-only").optimizations.arun(str(uuid4()), task=task)
