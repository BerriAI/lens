# Set it up for me

Use the [local quickstart](../deploy/lens/README.md#start-locally) to start Lens with Git and Docker. If you want your coding agent to configure the project as well, copy the applicable prompt below into a session with access to that project. These prompts are optional. They do not require a separate Lens setup tool

Lens runs with ClickHouse and includes its UI, API and background processing. Your agent keeps its existing model connection. Recording and inspecting traces does not require an analysis provider. The standalone quickstart is `docker compose up --wait` in a clone of this repository. It runs the published Lens image; `./deploy/lens/start --build` builds from source instead

## Start standalone Lens

```text
Set up standalone Lens for this project. Read https://github.com/BerriAI/lens/blob/main/deploy/lens/README.md and inspect the repository, running services, Docker availability, and existing telemetry configuration first. Reuse a working Lens deployment if present. Otherwise use the documented quickstart to start Lens and its bundled ClickHouse, recording the image and source commit used. Do not add a gateway or PostgreSQL. Ask only when a missing decision affects deployment location, access, existing data, or the agent to connect. Preserve existing configuration, secrets, model endpoints, and stored data; never print or commit credentials or rotate existing keys without a reason. Detect the agent framework, add the smallest supported instrumentation change, and keep telemetry credentials separate from model credentials. Use a Lens address reachable from the agent's actual network. Run a small real task and verify its receipt, exact trace ID, input, output, and any tool calls in Lens. Demo data and readiness alone do not count. If no runnable agent is present, verify a clearly labelled test trace and explain the remaining project integration. Report the UI URL, files changed, evidence, and how to stop and restart without deleting data. Add an analysis provider only if I ask for investigations, signals, or eval judging.
```

## Add Lens to existing LiteLLM

Use this when you already run LiteLLM and want Lens available from its dashboard. The agent must confirm that deployment's actual version and capabilities before changing it. Official paired Lens and LiteLLM releases share a version and are tested together. Lens remains independently deployable; verify API and UI compatibility when connecting an existing installation

```text
Add Lens to my existing LiteLLM deployment and make it accessible from the LiteLLM dashboard. First inspect this project and verify the gateway deployment, current versions, deployment method, existing Lens integration, storage, and credentials. If no active gateway is found, ask whether to locate the existing deployment or install standalone Lens. Read the current Lens documentation at https://github.com/BerriAI/lens and the integration guide for the installed LiteLLM version. Preserve gateway model routing, authentication, databases, unrelated deployment values, and existing Lens data. Use the supported native shared Lens UI integration and compatible API contracts; prefer the paired release for a new gateway integration, verify compatibility for existing installations, and preserve the embedded experience. Pin available artifacts by their documented version or digest and verify provenance where supported. If the required integration is only available in source, identify that boundary before choosing a source build or production upgrade. Configure a reachable Lens telemetry endpoint and a dedicated tracing key. For existing Lens metadata, follow the migration guide and verify a backup before any writer handoff. Prove that ordinary gateway inference still works, then send a real agent run and open that exact trace inside the LiteLLM dashboard. Verify input, output, tool calls, and the user's existing access scope. Report changes, artifact versions, trace evidence, and any remaining release prerequisite. Never expose secrets or delete existing volumes.
```

## Connect an agent to Lens already running

```text
Connect this project's agent to the Lens deployment I already have. Inspect the framework, current model connection, existing OpenTelemetry setup, and Lens connection configuration. Verify Lens readiness and access; do not reinstall services when the issue is authentication or routing. Ask for the Lens URL or a supported secret source only if it cannot be found safely, and never ask me to paste a secret into shared chat. Use https://github.com/BerriAI/lens/blob/main/deploy/lens/README.md#record-your-first-trace and the matching integration example. Preserve my model provider and reuse the existing tracer provider. Configure the dedicated tracing credential privately, retain any ingestion path prefix, and check reachability from the agent's network. Execute a small real task and verify the exact trace ID, input, output, tool calls, and agent name in Lens. Explain any missing content rather than counting a model response, demo data, or an upload HTTP status alone as success. Report the minimal changes and a link to the resulting trace without credentials.
```

## Connect Claude Code

Claude Code sends nothing to Lens by default. Follow [Claude Code](claude-code.md) to install the Lens plugin for Claude Code, or give your coding agent this prompt:

```text
Connect Claude Code on this machine to Lens with the LiteLLM Lens plugin for Claude Code. Read https://github.com/BerriAI/lens/blob/main/docs/claude-code.md first. Decide which setup applies: if Claude Code already sends model calls through a LiteLLM gateway connected to Lens (ANTHROPIC_BASE_URL and ANTHROPIC_AUTH_TOKEN are set), use that gateway address and the same LiteLLM key; otherwise use my Lens address and a Lens tracing key from an environment variable, and ask me for either only if it cannot be found safely. Run the two plugin commands from that page and pass the key through the environment variable that holds it, never the key itself. If CLAUDE_CODE_ENABLE_TELEMETRY and CLAUDE_CODE_ENHANCED_TELEMETRY_BETA are both set in the shell or in ~/.claude/settings.json, tell me that the plugin stays off until they are removed and ask whether to remove them. Tell me that the plugin stores my typed prompts, Claude Code's replies, the files it reads and the output of its commands in Lens. Then run one real Claude Code task and confirm a trace from the agent claude-code with one step per model call and its tool calls appears under Traces in Lens.
```

## Connect Codex

Codex sends nothing to Lens by default. Follow [Codex](codex-cli.md) to install the Lens plugin for Codex, or give your coding agent this prompt:

```text
Connect Codex on this machine to Lens with the LiteLLM Lens plugin for Codex. Read https://github.com/BerriAI/lens/blob/main/docs/codex-cli.md first. Decide which setup applies: if Codex already sends model calls through a LiteLLM gateway connected to Lens, use that gateway address and the same LiteLLM key; otherwise use my Lens address and a Lens tracing key, and ask me for either only if it cannot be found safely. Never print a key and never put its value in a command: pass the name of the environment variable that holds it with --key-env. Run the three install commands from that page with --agent-name codex-cli, and do not add an [otel] table to ~/.codex/config.toml. If one is already there and sends to the same Lens or gateway, tell me that it would record every chat a second time and ask whether to remove it; change nothing in it without my answer. If the plugin cannot run here (no Python 3.11 or newer, not macOS or Linux, or an address that is neither https nor localhost), stop and tell me, and offer the OpenTelemetry fallback from the same page. Tell me that the plugin stores my typed prompts, Codex's replies and the input and output of the commands Codex runs. Then run one real Codex task that runs a shell command and confirm in Lens > Traces one trace from the agent codex-cli with the prompt, a tool step and a model call step. Do not treat a readiness check as proof.
```

## Connect OpenCode

OpenCode 2.3.2 and later are recorded by the Lens plugin, and OpenCode 1.x by its own telemetry. Follow [OpenCode](opencode.md) for both, or give your coding agent this prompt:

```text
Connect OpenCode on this machine to Lens. Read https://github.com/BerriAI/lens/blob/main/docs/opencode.md first, then run opencode --version to pick the setup. Decide which address and key apply: if OpenCode already sends model calls through a LiteLLM gateway connected to Lens, use that gateway address and the same LiteLLM key; otherwise use my Lens address and a Lens tracing key, and ask me for either only if it cannot be found safely. Never print a key. On OpenCode 2.3.2 or later, run opencode plugin add github:BerriAI/litellm-lens-plugins, add LITELLM_LENS_OPENCODE_URL and LITELLM_LENS_OPENCODE_API_KEY to the shell profile I start opencode from, and run opencode service restart from a shell where both are set, because the plugin reads them from OpenCode's background service. On OpenCode 1.x, set experimental.openTelemetry to true in my opencode.json, preserving existing settings, and add OTEL_EXPORTER_OTLP_ENDPOINT, OTEL_EXPORTER_OTLP_HEADERS and OTEL_BSP_SCHEDULE_DELAY=1000 to that shell profile instead; if any OTEL_EXPORTER_OTLP_* variable is already set, stop and ask before changing it. Tell me that these settings store my typed prompts, the model's replies, the files OpenCode reads and the output of its commands. Then run one real OpenCode task that runs a shell command and confirm in Lens > Traces a trace from the agent opencode with a model step that has input and output and a tool step.
```

## Use an external ClickHouse deployment

The bundled ClickHouse is the default. Choose this variant only when you want to use storage you already operate. The supported external topology is one stable endpoint reaching one ClickHouse server with KeeperMap, shared by the Lens replicas. Load balancing across ClickHouse servers and automatic failover to a different server are unsupported. See the [storage requirements](../helm/lens/README.md#use-existing-clickhouse)

```text
Configure Lens to use my existing ClickHouse deployment. Inspect the current Lens deployment and read https://github.com/BerriAI/lens/blob/main/helm/lens/README.md and its linked storage configuration before changing anything. Confirm the ClickHouse version, database, TLS endpoint, authentication source, supported coordination/Keeper configuration, permissions, and network reachability. Reuse existing deployment and secret conventions; ask only for consequential missing storage or ownership details. Do not assume a managed ClickHouse service supports every required engine or coordination setting. If it cannot satisfy Lens's documented requirements, explain the exact incompatibility before proposing another deployment. Preserve existing tables, credentials and volumes. Use a separate verification database or the documented migration and backup procedure for populated Lens data. Configure Lens, send a trace, verify its content and a saved dataset, then restart Lens and confirm both records remain accessible. Report the verified configuration and any permissions or provider capabilities still required, without printing secrets.
```

## Enable investigations, signals, or eval judging

Tracing can be working already. This prompt adds only the model-backed function you ask for, after inspecting existing configuration

```text
Enable the Lens model-backed feature I am trying to use: investigations, signals, or eval judging. Inspect the existing Lens deployment, current provider configuration and relevant feature first. Read https://github.com/BerriAI/lens/blob/main/docs/analysis.md, https://github.com/BerriAI/lens/blob/main/docs/signals.md, or https://github.com/BerriAI/lens/blob/main/src/sdk/README.md as appropriate. Reuse a compatible configured provider or an existing LiteLLM model endpoint. Ask which feature, provider or spending limit I want only when that choice is missing and materially changes the setup. Use a currently supported model available to the account and keep provider secrets server-side using the deployment's secret mechanism. Preserve tracing, existing models, datasets, eval definitions and budgets. Explain which selected trace content will be sent to that provider. Apply the smallest configuration change, verify readiness, then perform one bounded real investigation, signal evaluation, or eval run using an existing suitable trace or case. For evals, verify the run, score and gate through the supported SDK/API and preserve the agent's own model configuration. Show the result and its evidence; a configured model list alone is not a successful provider call. Report actual cost when available and any remaining access or integration requirement without exposing credentials.
```

When a step fails, preserve completed work and diagnose that step. A connection failure, missing permission, unsupported version, and missing configuration require different fixes. Setup is complete only when the requested first result has been verified in the intended standalone or embedded Lens surface
