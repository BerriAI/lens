import json
import os
import socket
import subprocess
import sys
import time

import httpx
import pytest


def server(root, count):
    with socket.socket() as candidate:
        candidate.bind(("127.0.0.1", 0))
        port = candidate.getsockname()[1]
    dataset = root / "cases.json"
    dataset.write_text(
        json.dumps(
            {
                "dataset_id": "demo",
                "revision": 1,
                "cases": [
                    {
                        "id": f"case-{index}",
                        "messages": [{"role": "user", "content": f"Case {index}"}],
                        "meta": {"priority": "high" if index < 2 else "low"},
                        "source": {"finding_id": "1"},
                    }
                    for index in range(count)
                ],
            }
        )
    )
    process = subprocess.Popen(
        [sys.executable, "-m", "lens.cli", "dev-server", "--port", str(port), "--dataset-file", str(dataset)],
        env={**os.environ, "LENS_API_KEY": "lens-dev"},
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    address = f"http://127.0.0.1:{port}"
    try:
        for _ in range(100):
            try:
                if httpx.get(address + "/lens/datasets/resolve?name=demo", timeout=0.2).status_code == 401:
                    break
            except httpx.TransportError:
                time.sleep(0.05)
        else:
            raise AssertionError("Development server did not start")
        yield address
    finally:
        process.terminate()
        process.communicate(timeout=5)


@pytest.fixture(scope="module")
def endpoint(tmp_path_factory):
    yield from server(tmp_path_factory.mktemp("cli-server"), 3)


@pytest.fixture(scope="module")
def native_endpoint(tmp_path_factory):
    yield from server(tmp_path_factory.mktemp("sdk-server"), 36)
