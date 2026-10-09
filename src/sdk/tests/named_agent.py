import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from queue import Queue
from typing import Final

import httpx


def definition(name: str) -> dict[str, object]:
    return {
        "name": name,
        "updated_at": "2026-10-09T00:00:00Z",
        "spec": {
            "agent": "demo",
            "dataset_id": "demo",
            "revision": 1,
            "scorers": [{"kind": "task_completed"}],
            "trials": 1,
            "baseline": "main",
            "gate": {"pass_rate": 1},
            "timeout_per_trial_ms": 5000,
            "agent_io": {
                "version": 1,
                "connection": "agent",
                "submit": {
                    "method": "POST",
                    "path": "/fail" if name == "named-fail" else "/invoke",
                    "json": {"input": "", "request_id": ""},
                    "accepted_status": 200,
                },
                "input": [
                    {"source": "case.input", "target": "/input"},
                    {"source": "trial.request_id", "target": "/request_id"},
                ],
                "completion": {"kind": "immediate"},
                "output": {"pointer": "/output", "require_nonempty": True},
                "trace": {"source": "completed", "attribute": "session.id", "pointer": "/session_id"},
            },
        },
    }


def handler(endpoint: str, requests: Queue[tuple[str, str, str | None, bytes]]) -> type[BaseHTTPRequestHandler]:
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, format: str, *args: object) -> None:
            pass

        def reply(self, status: int, content: bytes) -> None:
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(content)))
            self.end_headers()
            self.wfile.write(content)

        def forward(self, method: str, body: bytes = b"") -> None:
            requests.put((method, self.path, self.headers.get("Authorization"), body))
            response: Final = httpx.request(
                method,
                endpoint + self.path,
                headers={
                    "Authorization": self.headers.get("Authorization", ""),
                    "X-Lens-Contract": self.headers.get("X-Lens-Contract", ""),
                    "Idempotency-Key": self.headers.get("Idempotency-Key", ""),
                    "Content-Type": "application/json",
                },
                content=body,
                timeout=10,
            )
            self.reply(response.status_code, response.content)

        def do_GET(self) -> None:
            if self.path.startswith("/lens/evals/named-"):
                requests.put(("GET", self.path, self.headers.get("Authorization"), b""))
                if self.headers.get("Authorization") != "Bearer lens-dev":
                    self.reply(401, b'{"code":"unauthorized"}')
                    return
                self.reply(200, json.dumps(definition(self.path.rsplit("/", 1)[-1])).encode())
                return
            self.forward("GET")

        def do_POST(self) -> None:
            body: Final = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            if self.path == "/lens/github/report":
                requests.put(("POST", self.path, self.headers.get("Authorization"), body))
                self.reply(
                    200,
                    json.dumps({"reports": [{"run_id": run_id} for run_id in json.loads(body)["run_ids"]]}).encode(),
                )
                return
            if self.path in {"/invoke", "/fail"}:
                requests.put(("POST", self.path, self.headers.get("Authorization"), body))
                value: Final = json.loads(body)
                self.reply(
                    200,
                    json.dumps(
                        {
                            "session_id": ("fail-" if self.path == "/fail" else "pass-") + value["request_id"],
                            "output": "Completed " + value["input"],
                        }
                    ).encode(),
                )
                return
            self.forward("POST", body)

        def do_PUT(self) -> None:
            self.forward("PUT", self.rfile.read(int(self.headers.get("Content-Length", "0"))))

    return Handler


def main() -> None:
    parser: Final = argparse.ArgumentParser(description="Named-eval HTTP fixture; uses synthetic dev-server scoring")
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--port", type=int, default=8766)
    args: Final = parser.parse_args()
    with ThreadingHTTPServer(("127.0.0.1", args.port), handler(args.endpoint, Queue())) as server:
        server.serve_forever()


if __name__ == "__main__":
    main()
