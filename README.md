# Lens agent-first experiment

Talk to Lens about your traces, or give a persistent bot an ongoing task. Home opens with a chat composer and a feed of reports, charts and evidence. The sidebar contains Home, New Session, Bots and Traces

This branch is an isolated experiment using native [Deep Agents](https://github.com/langchain-ai/deepagents) for ordinary sessions, [Hermes](https://github.com/NousResearch/hermes-agent) for persistent bots, and [AgentChat](https://github.com/BerriAI/agentchat) for Slack. All model calls and connected MCP tools go through LiteLLM

Traces remain immutable. Agents write typed annotations, reports and attachments in shared storage so the main agent can answer questions such as “Which failed runs did Request reviewer find?” Bots discover and reuse shared definitions, and can create new ones for new tasks

## Run locally

Use Docker with Compose 2.24 or newer. Export `LITELLM_PROXY_API_BASE` and `LITELLM_PROXY_API_KEY` through your shell or secret manager, using a dedicated key with the chosen model and MCP permissions. From this checkout:

```sh
./deploy/lens/start --build
```

Open [Lens](http://localhost:4318/ui/) and sign in with the generated `LENS_ADMIN_TOKEN` in `deploy/lens/.env`. See [Run the experiment](deploy/lens/README.md) for connecting traces, creating bots, Slack, deployment settings and restart verification

Use separate state volumes for this experiment. This branch requires both the Lens core image and native agent image built from the same source revision. For Kubernetes, use this checkout's [Helm chart](helm/lens/README.md#agent-workspace) with the agent workspace enabled; published Lens releases do not contain this experiment

## Development

The application backend is Rust in `src/worker`. The UI is in `src/ui/lib` and `src/ui/app`. The upstream agent runtimes are pinned external dependencies, with no custom Python backend

```sh
cargo test --manifest-path src/worker/Cargo.toml -p lens-agents
npm run typecheck:ui
npm exec --workspace @litellm/lens-ui -- vitest run \
  src/components/lens/LensWorkspace.integration.test.tsx \
  src/components/lens/LensNavigation.integration.test.tsx \
  src/components/lens/LensSetup.integration.test.tsx
```

The existing trace storage, ingestion, authentication and trace viewer remain. Specialized analysis, profile, dataset and evaluation routes are not mounted by this experiment's server

See [Contributing](CONTRIBUTING.md) for repository conventions
