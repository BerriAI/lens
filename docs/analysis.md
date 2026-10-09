# Configure analysis models

Lens can record and display traces before you configure a model. To run investigations, give the Lens server access to an analysis provider. Your agents keep their own model configuration

For optional help from your coding agent, copy the [provider setup prompt](setup-with-agent.md#enable-investigations-signals-or-eval-judging)

Investigations send selected trace content to the provider you choose. Each investigation has its own monthly spending limit. Provider credentials stay on the server and are never returned to the browser

## Add a provider

For the bundled Compose deployment, add the following settings to `deploy/lens/.env`. Other deployments supply the same environment variables through their secret configuration

The recommended OpenAI configuration uses the model name `analysis` in the dashboard. Replace `<your-provider-key>` with the key from your provider account and keep the file private:

```dotenv
OPENAI_API_KEY=<your-provider-key>
LENS_ANALYSIS_MODELS='[{"name":"analysis","model":"openai/gpt-6.1-sol","provider":"openai","api_key_env":"OPENAI_API_KEY","output_limits":{"max_tokens":4096}}]'
```

To use Anthropic instead, use its key and this configuration:

```dotenv
ANTHROPIC_API_KEY=<your-provider-key>
LENS_ANALYSIS_MODELS='[{"name":"analysis","model":"anthropic/claude-sonnet-5-5","provider":"anthropic","api_key_env":"ANTHROPIC_API_KEY","output_limits":{"max_tokens":4096}}]'
```

Use `api_key_env` to name the environment variable holding the credential. Do not put the key itself inside `LENS_ANALYSIS_MODELS`. The server rejects missing credentials and unknown configuration fields at startup

Apply the configuration from the repository root:

```sh
docker compose -f deploy/lens/compose.yaml up -d --wait
```

Open **Settings > Analysis** and select **Check configuration**. Your `analysis` model should appear, followed by **Analysis is configured** when the investigation worker is ready. This confirms configuration and worker readiness; your first investigation verifies provider access

## Run an investigation

After your agent sends a trace, open **Investigations > New investigation**. Select the activity to review, describe the expected behavior, choose `analysis`, and set a monthly spending limit. Run the investigation, then open its finding and follow an evidence citation to the original trace

The spend estimate reserves budget before each provider call. Successful calls record their cost in the run history, including calls that finish while an investigation is being cancelled. An exhausted budget stops new model calls; it does not stop trace ingestion

## Connect an OpenAI-compatible endpoint

Use this option for a compatible provider or an optional LiteLLM gateway. Set `LENS_MODEL_API_KEY` to its model credential and replace the example base URL with an address reachable from the Lens server:

```dotenv
LENS_MODEL_API_KEY=<your-model-credential>
LENS_ANALYSIS_MODELS='[{"name":"analysis","model":"my-analysis-model","provider":"openai_compatible","api_base":"https://models.example.com/v1","api_key_env":"LENS_MODEL_API_KEY","input_cost_per_token":0.000001,"output_cost_per_token":0.000002,"capacity":{"max_input_tokens":128000,"max_output_tokens":4096},"output_limits":{"max_tokens":4096}}]'
```

The example prices and token limits are placeholders. Replace them with your deployment's actual prices and supported limits. For known catalog models, omit those overrides to use Lens's bundled model metadata. The base URL ends at the API prefix, such as `/v1`, not `/chat/completions`. It is the model endpoint, separate from Lens's `/v1/traces` ingestion endpoint

## Resolve setup problems

| What you see | What to do |
| --- | --- |
| **Add an analysis provider** | Supply `LENS_ANALYSIS_MODELS` and its named credential to the Lens server, restart it, then select **Check configuration** |
| **Could not check analysis configuration** | Check that Lens is reachable and your session is valid, then retry. This does not mean the model configuration is missing |
| Models appear, but the worker is not ready | Check the Lens service logs and ClickHouse readiness, then retry |
| The run reports that the provider rejected the request | Check the provider key, access to the selected model, and any configured API base URL |
| The request exceeds the investigation budget | Increase that investigation's budget or reduce the model's output allowance, then run it again |
| The trace list is empty | Send a real run from your agent using a Lens tracing key. An analysis provider key does not authorize trace ingestion |

Changing a provider key requires updating its server-side secret and restarting Lens. Keep the existing admin token and ClickHouse configuration so sessions and saved work remain available
