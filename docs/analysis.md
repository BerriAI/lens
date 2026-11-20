# Configure analysis models

Lens can record and display traces before you configure a model. To run investigations, give the Lens server access to an analysis provider. Your agents keep their own model configuration

For optional help from your coding agent, copy the [provider setup prompt](setup-with-agent.md#enable-investigations-signals-or-eval-judging)

Investigations send selected trace content to the provider you choose. Each investigation has its own monthly spending limit. Provider credentials stay on the server and are never returned to the browser

## Connect a LiteLLM gateway

Set the gateway URL and a gateway virtual key on the Lens service, then restart it:

```dotenv
LENS_GATEWAY_API_BASE=https://your-gateway.example.com
LENS_GATEWAY_API_KEY=<gateway-virtual-key>
```

For Compose, put these values in `deploy/lens/.env`. For hosted services, use the service's environment and secret settings. Keep the key limited to models Lens should use; Lens fetches only the models accessible to that key from `/model_group/info`

Open **Settings > LiteLLM gateway** to check the connection and refresh the model list without restarting Lens. Discovered chat models are available for analysis when their pricing and output-token limits are valid. Each call starts with an output allowance of 16,384 tokens, or the model's smaller supported limit. When a response is cut off at that allowance, Lens repeats the call with a larger one, up to the model's output limit. Discovered evaluation models are available separately for [Signals](signals.md#connect-a-litellm-gateway)

A failed refresh shows an error and keeps the previous usable models. Explicit `LENS_ANALYSIS_MODELS` deployments remain available and take precedence over discovered aliases with the same name. Changing the gateway key or URL requires a restart; the browser never receives the key

`LENS_GATEWAY_URL` is a separate optional setting that tells Lens which analysis endpoint is the gateway, so Lens attaches its internal metadata only to calls sent there. It does not provide model discovery or replace the gateway API key. Every model call Lens makes names the agent `lens-signals` in its `User-Agent`, and Lens does not analyze traces from that agent

## Add a provider

For the bundled Compose deployment, add the following settings to `deploy/lens/.env`. Other deployments supply the same environment variables through their secret configuration

The recommended OpenAI configuration uses the model name `analysis` in the dashboard. Replace `<your-provider-key>` with the key from your provider account and keep the file private:

```dotenv
OPENAI_API_KEY=<your-provider-key>
LENS_ANALYSIS_MODELS='[{"name":"analysis","model":"openai/gpt-6.1-sol","provider":"openai","api_key_env":"OPENAI_API_KEY","output_limits":{"max_tokens":16384}}]'
```

To use Anthropic instead, use its key and this configuration:

```dotenv
ANTHROPIC_API_KEY=<your-provider-key>
LENS_ANALYSIS_MODELS='[{"name":"analysis","model":"anthropic/claude-sonnet-5-5","provider":"anthropic","api_key_env":"ANTHROPIC_API_KEY","output_limits":{"max_tokens":16384}}]'
```

Use `api_key_env` to name the environment variable holding the credential. Do not put the key itself inside `LENS_ANALYSIS_MODELS`. The server rejects missing credentials and unknown configuration fields at startup

`output_limits.max_tokens` is the output allowance each call starts with, and it sets how much budget a call reserves. When a response is cut off at that allowance, Lens repeats the call with a larger one, up to `capacity.max_output_tokens` or the model's known output limit. Set `capacity.max_output_tokens` to the same value as `max_tokens` to keep a fixed cap

Apply the configuration from the repository root:

```sh
docker compose -f deploy/lens/compose.yaml up -d --wait
```

Open **Settings > Analysis** and select **Check configuration**. Your `analysis` model should appear, followed by **Analysis is configured** when the investigation worker is ready. This confirms configuration and worker readiness; your first analysis run verifies provider access

## Automatic analysis in Findings

Once an analysis model is connected, Lens creates an automatic analysis configuration for each agent discovered in production traces. The first run waits for 10 eligible traces from the last 24 hours. Traces settle for two minutes after receipt so incomplete runs are not analyzed immediately. Evaluation traffic and browser demo data do not create automatic analysis configurations

Open **Findings** to see received trace counts, waiting or running status, the last result, and the next scheduled time. Select **Configure** beside an agent to choose its model, edit what to look for, change its frequency or monthly budget, or pause analysis

Defaults are one analysis every 60 minutes, up to 10 traces per run, and a $10 monthly budget per agent. After the first run, the existing scheduler reviews new activity in the saved lookback window. The interval begins after a run finishes. Existing agent configurations, including paused configurations, are preserved; a configuration covering all agents also prevents automatic duplicates

New automatic analyses use the first explicit `LENS_ANALYSIS_MODELS` deployment when configured, otherwise the first available discovered analysis model. Gateway discovery does not replace an agent's saved model selection

Open a finding and follow an evidence citation to the original trace. Existing analysis records and history are preserved. Saved investigation links open Findings

To receive new improvement candidates in Slack, configure the optional [Slack findings integration](slack.md)

Lens marks the stable start of each prompt and the latest turns for prompt caching, so the repeated context in a long investigation is billed at the provider's cached rate. This applies to Anthropic models called directly and to Anthropic models served by a gateway under their standard names, such as `anthropic/claude-haiku-4-5`. OpenAI models cache automatically. For a gateway model, Lens charges cached tokens at the catalog's cache rates when the gateway reports the catalog's input price, and at the full input price otherwise

Lens asks providers that accept a JSON schema to enforce the response format, and reads each response as a stream so a load balancer in front of the provider sees data arriving while the model writes. A provider that rejects that request is asked again with a buffered JSON mode request, which keeps the cache markers. A gateway that rejects the cache markers is asked again without them and keeps the schema and the stream. When a gateway times out before a response starts, which happens when a reasoning model thinks for longer than the gateway waits, Lens asks again with `reasoning_effort` set to `low`. Lens keeps whichever form worked for that deployment until it restarts or the gateway model list is refreshed. Every quote in a finding is checked against the recorded span. A quote that differs from the recording only in whitespace, escaping, emphasis markers or an ellipsis is saved as the recorded text. A quote that cannot be found is returned to the model with the nearest recorded passage, and a result that still cannot be verified is discarded while the verified results are kept

A run that finishes while some tasks failed is shown as **Partial**. Its verified findings are saved, and the failed traces are analyzed again on the next run

The spend estimate reserves budget before each provider call. Successful calls record their cost in the run history, including calls that finish while analysis is being cancelled. An exhausted budget stops new model calls; it does not stop trace ingestion

## Connect an OpenAI-compatible endpoint

Use this option for a compatible provider or an optional LiteLLM gateway. Set `LENS_MODEL_API_KEY` to its model credential and replace the example base URL with an address reachable from the Lens server:

```dotenv
LENS_MODEL_API_KEY=<your-model-credential>
LENS_ANALYSIS_MODELS='[{"name":"analysis","model":"my-analysis-model","provider":"openai_compatible","api_base":"https://models.example.com/v1","api_key_env":"LENS_MODEL_API_KEY","input_cost_per_token":0.000001,"output_cost_per_token":0.000002,"capacity":{"max_input_tokens":128000,"max_output_tokens":32768},"output_limits":{"max_tokens":16384}}]'
```

The example prices and token limits are placeholders. Replace them with your deployment's actual prices and supported limits. For known catalog models, omit those overrides to use Lens's bundled model metadata. The base URL ends at the API prefix, such as `/v1`, not `/chat/completions`. It is the model endpoint, separate from Lens's `/v1/traces` ingestion endpoint

## Resolve setup problems

| What you see | What to do |
| --- | --- |
| **Add an analysis provider** | Supply `LENS_ANALYSIS_MODELS` and its named credential to the Lens server, restart it, then select **Check configuration** |
| **Could not check analysis configuration** | Check that Lens is reachable and your session is valid, then retry. This does not mean the model configuration is missing |
| Models appear, but the worker is not ready | Check the Lens service logs and ClickHouse readiness, then retry |
| The run reports that the provider rejected the request | Check the provider key, access to the selected model, and any configured API base URL |
| The request exceeds the investigation budget | Increase the analysis budget or reduce the model's output allowance, then run it again |
| A task reports that the response reached the model output limit | Raise `capacity.max_output_tokens` for that deployment or choose a model with a larger output limit |
| The run reports that the provider request timed out | A proxy or load balancer between Lens and the provider closed the connection. Raise its idle timeout above the time your model needs before it starts answering, or choose a model that reasons less |
| The trace list is empty | Send a real run from your agent using a Lens tracing key. An analysis provider key does not authorize trace ingestion |

Changing a provider key requires updating its server-side secret and restarting Lens. Keep the existing admin token and ClickHouse configuration so sessions and saved work remain available
