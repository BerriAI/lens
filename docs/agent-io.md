# Save an agent input/output contract

A saved eval defines which dataset to run, how to call the agent, and how to score its answer. The SDK executes the agent requests on your machine or CI runner. Save the definition once, then call `lens.evals.run("agent-regressions")` from an ordinary test

Use a server and [SDK](../src/sdk/README.md#run-a-saved-eval) that support contract 2. The API accepts the definition below at `PUT /lens/evals/{name}` with `X-Lens-Contract: 2`. Contract 1 keeps its existing trace-only response shapes and cannot write agent I/O contracts or direct outputs. The JSON declaration's own `version` is `1`

## An agent that returns its answer immediately

Save this as `eval.json`, replacing `dataset-id` with an existing Lens dataset ID and `revision` with the revision you want to run

```json
{
  "agent": "coding-agent",
  "dataset_id": "dataset-id",
  "revision": 1,
  "scorers": [
    {"kind": "judge", "prompt": "Does the answer satisfy every expected outcome for this case?"}
  ],
  "baseline": "main",
  "trials": 1,
  "gate": {"regressions": 0, "critical": 0, "pass_rate": 1.0, "min": {"judge": 1.0}},
  "timeout_per_trial_ms": 600000,
  "agent_io": {
    "version": 1,
    "connection": "coding-agent",
    "submit": {
      "method": "POST",
      "path": "/invoke",
      "accepted_status": 200,
      "json": {"input": "", "request_id": ""}
    },
    "input": [
      {"source": "case.input", "target": "/input"},
      {"source": "trial.request_id", "target": "/request_id"}
    ],
    "completion": {"kind": "immediate"},
    "output": {"pointer": "/output", "require_nonempty": true}
  }
}
```

The agent receives `{"input":"the case input","request_id":"a stable trial ID"}` and must return HTTP 200 with a JSON string at `/output`, for example `{"output":"The completed answer"}`

Configure its trusted local connection in the agent repository's `pyproject.toml`

```toml
[tool.lens.connections.coding-agent]
base_url_env = "AGENT_BASE_URL"
auth = "bearer"
token_env = "AGENT_API_KEY"
```

Set `AGENT_BASE_URL` to the agent origin, such as `https://agent.example.com`, and provide `AGENT_API_KEY` through your secret manager. Use `auth = "none"` and omit `token_env` for an endpoint that requires no authentication

Save the definition with a Lens credential that can write evals

```sh
curl --fail-with-body \
  -X PUT "$LENS_BASE_URL/lens/evals/agent-regressions" \
  -H "Authorization: Bearer $LENS_API_KEY" \
  -H "X-Lens-Contract: 2" \
  -H "Content-Type: application/json" \
  --data-binary @eval.json
```

The response contains `name`, `spec`, and `updated_at`. Read it back with `GET /lens/evals/agent-regressions` using the same headers. Saving or reading a definition does not start agent tasks

## Moyai's asynchronous task API

For a Moyai eval, keep the dataset, revision, gate, and timeout fields above. Set `agent` to `moyai`, replace `agent_io` with the following object, and save it under `/lens/evals/moyai-coding-regressions`

```json
{
  "version": 1,
  "connection": "moyai",
  "submit": {
    "method": "POST",
    "path": "/api/runs",
    "accepted_status": 201,
    "json": {
      "prompt": "",
      "mode": "modal",
      "chat_enabled": true,
      "repo_url": "",
      "plugins": [],
      "environment_id": "none",
      "client_id": ""
    }
  },
  "input": [
    {"source": "case.input", "target": "/prompt"},
    {"source": "trial.request_id", "target": "/client_id"}
  ],
  "completion": {
    "kind": "poll",
    "id_pointer": "/id",
    "path_template": "/api/runs/{id}",
    "status_pointer": "/status",
    "success": ["completed", "idle"],
    "failure": ["failed", "cancelled", "interrupted"],
    "error_pointer": "/error",
    "interval_ms": 1000,
    "timeout_ms": 600000,
    "require_item": {
      "array_pointer": "/messages",
      "matches": {"role": "assistant", "status": "completed"}
    }
  },
  "output": {"pointer": "/summary", "require_nonempty": true},
  "trace": {"source": "accepted", "attribute": "session.id", "pointer": "/id"}
}
```

The SDK submits a fresh chat session, retains its accepted ID, and polls that session until a success status and completed assistant message are both present. A nonempty summary while Moyai is `saving` is not a completed answer. The stable `/client_id` preserves Moyai's submission identity

Use a dedicated Moyai deployment and configure its local profile

```toml
[tool.lens.connections.moyai]
base_url_env = "MOYAI_EVAL_URL"
auth = "moyai_session"
password_env = "MOYAI_EVAL_PASSWORD"
```

The preset handles password login, the session cookie, Origin, and CSRF. The deployment must permit password authentication and have a working model gateway and execution worker. Successful login does not prove that execution or trace export is ready

For this trace mapping, the Moyai server must independently stamp `agent.name = "moyai"`, `agent.version` matching the evaluated deployment's build SHA, and `deployment.environment = "lens-eval"`. The session's trace attribute must be the exact accepted `session.id`. Existing tracing alone does not guarantee those version and environment fields. Verify the exported attributes in your deployment before enabling this mapping. Set the SDK's `LENS_VERSION` to the same deployed build, or pass an explicit `Execution`; this does not change the agent's instrumentation

If your deployment does not yet emit those attributes, omit the optional `trace` mapping and use only judge scorers. The completed HTTP output can be evaluated without trace scoring or cost gates

Lens validates the matched traces; supplying output alongside a trace does not bypass version, environment, or trace completion checks. Trace delivery can lag the agent's completed HTTP response

## Mapping and scoring rules

The exact generated types are in [the shared schema](../schema/lens.v1.json). An agent I/O declaration requires `version`, `connection`, `submit`, `input`, `completion`, and `output`; `trace` is optional. Request methods are POST, accepted statuses are 200–299, and polling uses GET with HTTP 200

Input sources are `case.input`, `case.followups`, and `trial.request_id`. A `case.input` binding is required. Targets use RFC 6901 JSON Pointers, not JSONPath or executable expressions: `/messages/0/content` selects an existing array item, `~1` escapes `/`, and `~0` escapes `~`. Targets must exist in the fixed JSON body and select scalar or empty-array leaves. Duplicate and ancestor-overlapping targets are rejected. Cases with follow-ups require an explicit `case.followups` mapping; otherwise that case fails before task submission

Polling paths contain exactly one `{id}` placeholder. The accepted ID must be a nonempty string and is encoded as one URL path segment. Success and failure states must be nonempty, unique, and disjoint. `interval_ms` is 250–60,000, `timeout_ms` is 1,000–3,600,000, and the interval cannot exceed the timeout. A shorter poll timeout cannot extend the eval's per-trial deadline. `require_item` optionally requires an array member matching every declared scalar field

Output must resolve to a JSON string. `require_nonempty: true` rejects empty or whitespace-only text. With `false`, an empty string is a real result to grade, distinct from a missing field. An optional trace mapping reads a nonempty `session.id` or `trace_id` from the `accepted` or `completed` response

Output-only evals support judge scorers. `task_completed`, `called_before`, and cost gates require a trace mapping because output text cannot establish tool execution or spend. To add trace checks to the Moyai definition, include scorers such as `{"kind":"task_completed"}` alongside its judge and retain the trace mapping

The saved contract contains no credentials or environment variable names. It selects a trusted local profile, whose credentials stay separate from `LENS_API_KEY` and the agent's tracing key. Requests remain on that profile's origin and do not follow redirects. The SDK does not import or execute code from saved definitions, and it does not automatically retry an uncertain task submission

Unexpected HTTP statuses, missing fields, wrong types, failed agent states, and timeouts become explicit trial errors. Missing definitions or local configuration fail the named run. A quality-gate failure still returns a completed report; call `report.assert_passed()` to fail your test

The Moyai preset attempts one authenticated cancellation after an accepted job exceeds a deadline, with a separate five-second cleanup limit. Cancellation failure remains visible in the trial error. Generic polling contracts have no cancellation endpoint; their remote jobs may continue after the SDK stops waiting

Fresh local calls receive fresh execution identities. GitHub runs use the run ID and attempt. If you deliberately reuse an execution identity after a process crash, a still-running Lens trial may be submitted again with the same `trial.request_id`. The agent must deduplicate that ID to make resumption safe; the SDK does not promise exactly-once remote execution
