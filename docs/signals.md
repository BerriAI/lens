# Configure signals

Signals flag traces that match questions you define, such as whether an agent reports success without confirming the result. Lens sends eligible trace content to a configured evaluation provider, then displays matches in **Traces**

Signals use a Decisions API evaluation model. An investigation analysis model alone does not enable them

For optional help from your coding agent, copy the [provider setup prompt](setup-with-agent.md#enable-investigations-signals-or-eval-judging) and specify signals

## Connect a LiteLLM gateway

Set these environment variables on the Lens service and restart it:

```dotenv
LENS_GATEWAY_API_BASE=https://your-gateway.example.com
LENS_GATEWAY_API_KEY=<gateway-virtual-key>
```

Lens fetches the models accessible to this key from the gateway's `/model_group/info` endpoint. Open **Settings**, check the gateway connection, and refresh its models whenever the gateway configuration changes. The key stays on the server

In **Settings > Signals**, choose a discovered System 1 model and save your questions. Only models reported with `mode: evaluation` appear in this selector; Lens sends their requests through the gateway's `/v1/decisions` endpoint using the exact model alias. Ordinary chat models are available for analysis when the gateway supplies valid pricing and output-token limits

Discovered analysis models request at most 4,096 output tokens per call (or their smaller supported limit), while retaining their full context capacity for input checks and budget accounting

An unsuccessful refresh shows the connection error and keeps the last usable models. A successful refresh replaces the discovered list. Explicit `LENS_ANALYSIS_MODELS` and `LENS_EVALUATION_MODELS` remain available and take precedence over a discovered alias with the same name

These API connection variables are separate from `LENS_GATEWAY_URL` and `LENS_GATEWAY_SECRET`, which optionally sign internal Lens traffic. Do not use the signing secret as a gateway API key

## Add an evaluation provider

Supply a TypeSafe API key as `TYPESAFE_API_KEY` to the Lens service, then set:

```sh
export LENS_EVALUATION_MODELS='[{"name":"signals","model":"typesafe/jev-latest","provider":"typesafe","api_key_env":"TYPESAFE_API_KEY"}]'
```

For the bundled Compose deployment, add the settings to `deploy/lens/.env`, without `export`:

```dotenv
TYPESAFE_API_KEY=<your-provider-key>
LENS_EVALUATION_MODELS='[{"name":"signals","model":"typesafe/jev-latest","provider":"typesafe","api_key_env":"TYPESAFE_API_KEY"}]'
```

Replace `<your-provider-key>` with the key from your provider account. Keep the environment file private. Apply the change from the repository root:

```sh
docker compose -f deploy/lens/compose.yaml up -d --wait
```

Open **Settings > Signals**, choose `signals`, select the questions to evaluate and save. Run your agent and open its trace to check the resulting flags. Provider calls can incur charges and include captured trace content

The UI's threshold controls which scores appear as flags. Configure questions that produce a useful yes/no signal and check the original trace before acting on a result

## Use a compatible connection

Lens also accepts `perplexity`, `openrouter`, `cloudflare`, `strands_decider`, and `decisions_compatible` provider configurations. Use a model that supports the provider's Decisions API. Cloudflare and Strands require `api_base`; compatible connections need the base URL reachable from Lens

For a compatible Decisions service:

```sh
export LENS_EVALUATION_MODELS='[{"name":"signals","model":"your-evaluation-model","provider":"decisions_compatible","api_base":"https://evaluations.example.com/v1","api_key_env":"EVALUATION_API_KEY"}]'
```

Replace the model and API base with those from your service, and supply `EVALUATION_API_KEY` through your secret configuration. Do not use an ordinary chat-completions endpoint as a Decisions endpoint

## Resolve setup problems

If the model does not appear, confirm that the Lens process received `LENS_EVALUATION_MODELS` and its named credential, then restart it and refresh Settings. An invalid deployment configuration fails at startup rather than silently disabling evaluation

If Lens cannot load signal settings, check the service and ClickHouse logs. A failed settings request does not establish that the evaluation model is missing

If a trace is unflagged, inspect the configured questions and threshold, confirm that the trace has captured content, and check the service logs for provider failures. An unflagged trace is not proof that every question was successfully evaluated
