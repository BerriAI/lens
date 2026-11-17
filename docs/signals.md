# Configure signals

Signals flag traces that match questions you define, such as whether an agent reports success without confirming the result. Lens sends eligible trace content to a configured evaluation provider, then displays matches in **Traces**

Signals read every page and preserve each step's full captured content. The evaluation provider's context window still applies; a provider failure is recorded as a failed attempt

Signals use a Decisions API evaluation model by default. A chat model configured for analysis can judge traces instead, which helps when traces are longer than your evaluation model accepts. See [Use a chat model as the judge](#use-a-chat-model-as-the-judge)

For optional help from your coding agent, copy the [provider setup prompt](setup-with-agent.md#enable-investigations-signals-or-eval-judging) and specify signals

## Connect a LiteLLM gateway

Set these environment variables on the Lens service and restart it:

```dotenv
LENS_GATEWAY_API_BASE=https://your-gateway.example.com
LENS_GATEWAY_API_KEY=<gateway-virtual-key>
```

Lens fetches the models accessible to this key from the gateway's `/model_group/info` endpoint. Open **Settings**, check the gateway connection, and refresh its models whenever the gateway configuration changes. The key stays on the server

In **Settings > Signals**, choose a discovered System 1 model and save your questions. Models reported with `mode: decisions` or the legacy `mode: evaluation` appear in this selector; Lens sends their requests through the gateway's `/v1/decisions` endpoint using the exact model alias. Ordinary chat models are available for analysis when the gateway supplies valid pricing and output-token limits

Discovered analysis models start each call with an output allowance of 16,384 tokens (or their smaller supported limit) and raise it only after a cut-off response, while retaining their full context capacity for input checks and budget accounting

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

Click a signal in the trace list or run header to open the step containing its supporting text. Lens highlights the exact captured passage as a **Flagged excerpt**, so you can inspect what the classifier used to locate the issue

Evidence selection starts alongside classification in the same Decisions request. Each signal keeps its yes/no score and adds a Choice question over identified passages, including a no-evidence option. Each question contains at most 255 options. For more than 254 passages, Lens asks the model to choose a range, then narrows only the selected ranges until it identifies individual passages. Later passages remain selectable. Large traces can require additional model calls, sharing the original 60-second inference deadline. If a later evidence lookup fails, Lens keeps the completed scores and any passages already found. Lens accepts only passages from the trace content supplied to the model

Signals inspect bounded excerpts, so a highlighted passage is a location to investigate rather than proof of a root cause. Captured text can differ from rendered Markdown or JSON; Lens shows the original excerpt without inventing line numbers for the formatted output. Older classifications and results without a usable passage show an evidence-unavailable message

## Use a chat model as the judge

Decisions models are fast and cheap, but their input window is small for long, tool-heavy runs. When a trace does not fit, Lens first drops evidence selection and then shortens steps, so the part that shows the failure can be cut. A long-context chat model reads the whole trace instead

Any model available for analysis can be the judge. Configure it as described in [Configure analysis models](analysis.md), then choose it in **Settings > Signals**. Chat models are listed after the System 1 models and are marked **Chat judge**. Nothing else changes: each signal keeps its question, its score from 0 to 1, the threshold and the flagged excerpt

Lens sends the judge the same labeled trace content and questions in one request and asks for every score and supporting excerpt at once. It accepts only scores between 0 and 1 and excerpts that were part of the request. A trace that is still too long for the chat model is reduced the same way as for a Decisions model, and the result is marked partial. A reply Lens cannot use, such as one cut off at the output limit, fails that trace until the trace or the signal configuration changes, so a long trace is not sent again every half hour

Choosing a chat model is a cost decision. Every trace is sent in full to that model, so a 250,000 token trace costs 250,000 input tokens per classification, and a trace is classified again when it grows. Signals have no spending limit of their own. Pick a model whose input price suits your trace volume and use the spending controls of your provider or gateway key. A chat judge gets up to four minutes per trace instead of one

A chat score is the model's own estimate. It is not calibrated like a Decisions score, so check a few flagged and unflagged traces before relying on a threshold

## Use a compatible connection

Lens also accepts `perplexity`, `openrouter`, `cloudflare`, `strands_decider`, and `decisions_compatible` provider configurations. Use a model that supports the provider's Decisions API. Cloudflare and Strands require `api_base`; compatible connections need the base URL reachable from Lens

For a compatible Decisions service:

```sh
export LENS_EVALUATION_MODELS='[{"name":"signals","model":"your-evaluation-model","provider":"decisions_compatible","api_base":"https://evaluations.example.com/v1","api_key_env":"EVALUATION_API_KEY"}]'
```

Replace the model and API base with those from your service, and supply `EVALUATION_API_KEY` through your secret configuration. Do not configure an ordinary chat-completions endpoint as a Decisions endpoint. To judge with a chat model, add it as an analysis model instead

## Resolve setup problems

If the model does not appear, confirm that the Lens process received `LENS_EVALUATION_MODELS` and its named credential, then restart it and refresh Settings. An invalid deployment configuration fails at startup rather than silently disabling evaluation

If Lens cannot load signal settings, check the service and ClickHouse logs. A failed settings request does not establish that the evaluation model is missing

If a trace is unflagged, inspect the configured questions and threshold, confirm that the trace has captured content, and check the service logs for provider failures. An unflagged trace is not proof that every question was successfully evaluated

Provider outages, rate limits, and timeouts pause new work for the same signal configuration across workers. The pause starts at one minute and doubles after repeated failures, up to 30 minutes; a longer provider `Retry-After` is honored. Requests already in flight can finish, and a successful recovery resets the pause. Invalid-request and request-size refusals (HTTP 400, 413, or 422) are not retried for an unchanged trace and configuration. Those refusals and incomplete model answers do not start a shared pause. Correct the request or change the signal configuration before retrying permanently refused traces. Authentication and endpoint failures use the shared cooldown so workers can recover after their provider configuration is repaired
