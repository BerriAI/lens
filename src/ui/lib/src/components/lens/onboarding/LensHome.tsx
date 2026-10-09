"use client";

import { useState } from "react";
import { ArrowRight, Pencil } from "lucide-react";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../ui/select";
import type { LensAgents } from "../agents/AgentScoped";
import { useLensAccessToken } from "../data/LensServices";
import { useConnectProjectRoute, type ProjectConnection } from "../route";
import { useOnboarding } from "./OnboardingContext";
import { HomeConnectionStatus } from "./HomeConnectionStatus";
import {
  CodeBlock,
  CodingAgentSetup,
  maskSecret,
  projectSetupPrompt,
  TracingKey,
  useLensService,
} from "./tracing/TracingSetupCard";

export function LensHome({
  agents,
  enabled,
  onOpenAgent,
  onOpenAgents,
  onSetup,
}: {
  readonly agents: LensAgents;
  readonly enabled: boolean;
  readonly onOpenAgent: (name: string) => void;
  readonly onOpenAgents: () => void;
  readonly onSetup?: () => void;
}) {
  const [project, setProject] = useConnectProjectRoute();
  const [name, setName] = useState(project?.name ?? "");
  const [integration, setIntegration] = useState<ProjectConnection["integration"]>(project?.integration ?? "auto");
  const [keyStatus, setKeyStatus] = useState<(ProjectConnection & { readonly active: boolean }) | null>(null);
  const keyReady =
    keyStatus?.name === project?.name && keyStatus?.integration === project?.integration ? keyStatus?.active : null;
  return (
    <section aria-label="Get started with Lens" className="mx-auto w-full max-w-6xl px-3 py-8 sm:px-6 sm:py-10">
      <div className="mb-7 flex flex-wrap items-start justify-between gap-4">
        <div className="space-y-2">
          <h2 className="text-2xl font-semibold tracking-tight">Connect your project</h2>
          <p className="text-sm text-muted-foreground">
            Send your agent’s traces to Lens. We’ll confirm when they arrive.
          </p>
        </div>
        {agents.list.agents.length > 0 && (
          <Button variant="outline" size="sm" onClick={onOpenAgents}>
            View agents
            <ArrowRight aria-hidden className="size-3.5" />
          </Button>
        )}
      </div>
      {!enabled ? (
        <p className="text-sm text-muted-foreground">Turn off demo data to connect your project.</p>
      ) : (
        <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_320px]">
          <div className="min-w-0">
            {project ? (
              <>
                <div className="mb-4 flex items-center justify-between gap-3 rounded-lg border bg-muted/20 px-4 py-3">
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium">{project.name}</p>
                    <p className="mt-0.5 text-xs text-muted-foreground">
                      {project.integration === "moyai"
                        ? "Moyai · tracing included"
                        : "Claude Code or Codex can detect your framework"}
                    </p>
                  </div>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      setName(project.name);
                      setIntegration(project.integration);
                      setProject(null);
                      setKeyStatus(null);
                    }}
                  >
                    <Pencil aria-hidden className="size-3.5" />
                    Change
                  </Button>
                </div>
                <ProjectInstructions
                  key={`${project.name}:${project.integration}`}
                  project={project}
                  onKeyReady={(active) => setKeyStatus({ ...project, active })}
                />
              </>
            ) : (
              <form
                className="space-y-5 rounded-xl border bg-card p-5 sm:p-6"
                onSubmit={(event) => {
                  event.preventDefault();
                  if (name.trim()) setProject({ name: name.trim(), integration });
                }}
              >
                <div className="space-y-1">
                  <h3 className="font-medium">What are you connecting?</h3>
                  <p className="text-sm text-muted-foreground">
                    Your agent will appear automatically when it sends its first trace.
                  </p>
                </div>
                <div className="space-y-2">
                  <label htmlFor="home-agent-name" className="text-sm font-medium">
                    Agent name
                  </label>
                  <Input
                    id="home-agent-name"
                    placeholder="e.g. moyai"
                    value={name}
                    onChange={(event) => setName(event.target.value)}
                    maxLength={128}
                    required
                  />
                  <p className="text-xs text-muted-foreground">
                    Already instrumented? Use the name your app sends with its traces.
                  </p>
                </div>
                <div className="space-y-2">
                  <label id="home-integration-label" className="text-sm font-medium">
                    Your project
                  </label>
                  <Select
                    value={integration}
                    onValueChange={(value) => {
                      if (value !== "auto" && value !== "moyai") return;
                      setIntegration(value);
                      if (value === "moyai" && !name.trim()) setName("moyai");
                    }}
                  >
                    <SelectTrigger aria-labelledby="home-integration-label" className="w-full">
                      <SelectValue>{integration === "auto" ? "Any agent or framework" : "Moyai"}</SelectValue>
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="auto">Any agent or framework</SelectItem>
                      <SelectItem value="moyai">Moyai</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <Button type="submit" disabled={!name.trim()}>
                  Connect project
                  <ArrowRight aria-hidden className="size-4" />
                </Button>
              </form>
            )}
          </div>
          <HomeConnectionStatus
            name={project?.name ?? ""}
            enabled={enabled}
            keyReady={keyReady}
            onOpenTraces={onOpenAgent}
            onSetup={onSetup}
          />
        </div>
      )}
      {onSetup && (
        <Button variant="link" size="sm" className="mt-5 px-0 text-xs text-muted-foreground" onClick={onSetup}>
          Deployment setup
        </Button>
      )}
    </section>
  );
}

const shellValue = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

function ProjectInstructions({
  project,
  onKeyReady,
}: {
  readonly project: ProjectConnection;
  readonly onKeyReady: (ready: boolean) => void;
}) {
  const accessToken = useLensAccessToken();
  const { canMintTracingKey, readOnly } = useOnboarding();
  const connection = useLensService(accessToken);
  const [tracingKey, setTracingKey] = useState<string | null>(null);
  const [useExistingKey, setUseExistingKey] = useState(false);
  const [endpointOverride, setEndpointOverride] = useState<string | null>(null);
  const [copiedInstructions, setCopiedInstructions] = useState(false);
  const traceUrl = connection.data?.url?.replace(/\/$/, "") ?? "";
  const endpoint = endpointOverride ?? `${traceUrl}/v1/traces`;
  const native = project.integration === "moyai";
  const ready = Boolean(connection.data?.connected && connection.data.status.storage_ready && traceUrl);
  const hasKey = tracingKey !== null || useExistingKey;
  const validEndpoint = validTraceEndpoint(endpoint, native);
  const environment = (secret: string | null) =>
    native
      ? `LITELLM_TRACE_ENDPOINT=${shellValue(endpoint)}\nLITELLM_TRACE_API_KEY=${shellValue(secret ?? "<your tracing key>")}`
      : `export LITELLM_TRACING_KEY=${shellValue(secret ?? "<your tracing key>")}\nexport OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=${shellValue(endpoint)}\nexport OTEL_EXPORTER_OTLP_TRACES_HEADERS="Authorization=Bearer $LITELLM_TRACING_KEY"\nexport OTEL_EXPORTER_OTLP_PROTOCOL="http/protobuf"`;
  const instructions = `${projectSetupPrompt(traceUrl)}\n\nThe agent name to look for in Lens is ${JSON.stringify(project.name)}. For new instrumentation, use this name in gen_ai.agent.name. If an already-instrumented app uses a different name, report it so I can select that name in Lens.`;
  if (connection.isPending)
    return (
      <p role="status" className="py-6 text-sm text-muted-foreground">
        Checking the Lens connection…
      </p>
    );
  if (!ready)
    return (
      <div role="alert" className="rounded-xl border p-6 text-sm text-muted-foreground">
        Lens needs a working API and trace storage before you can connect. Check the live connection panel for the next
        action.
      </div>
    );
  return (
    <div className="space-y-5">
      <section className="space-y-4 rounded-xl border bg-card p-5 sm:p-6" aria-label="Project credentials">
        <div className="space-y-1">
          <h3 className="font-medium">{hasKey ? "Save this in your project" : "Get a tracing key"}</h3>
          <p className="text-sm text-muted-foreground">
            {hasKey
              ? native
                ? "Add these values to Moyai’s environment, then restart it."
                : "Set these environment variables where your agent runs."
              : "A dedicated key lets your project send traces to Lens."}
          </p>
        </div>
        {!useExistingKey && canMintTracingKey && !readOnly && (
          <TracingKey
            accessToken={accessToken}
            tracingKey={tracingKey}
            compact
            name={project.name}
            onCreated={(key, active) => {
              setTracingKey(key);
              onKeyReady(active);
            }}
          />
        )}
        {!hasKey && (
          <>
            {(!canMintTracingKey || readOnly) && (
              <p className="text-sm text-muted-foreground">Ask your Lens administrator for a tracing key.</p>
            )}
            <Button variant="link" size="sm" className="px-0" onClick={() => setUseExistingKey(true)}>
              I already have a tracing key
            </Button>
          </>
        )}
        {hasKey && (
          <>
            {native && (
              <div className="space-y-2">
                <label htmlFor="home-trace-endpoint" className="text-sm font-medium">
                  HTTPS traces endpoint
                </label>
                <Input
                  id="home-trace-endpoint"
                  type="url"
                  value={endpoint}
                  onChange={(event) => setEndpointOverride(event.target.value)}
                />
              </div>
            )}
            {validEndpoint ? (
              <CodeBlock
                code={environment(tracingKey)}
                display={environment(tracingKey && maskSecret(tracingKey))}
                copyLabel="Copy project environment"
                wrap
              />
            ) : (
              <p role="alert" className="text-sm text-destructive">
                {native
                  ? "Moyai requires an HTTPS endpoint ending in /v1/traces. Use an address reachable from your app."
                  : "Check the public Lens address in deployment setup. Traces need an HTTP or HTTPS endpoint ending in /v1/traces."}
              </p>
            )}
            {useExistingKey && (
              <p className="text-xs text-muted-foreground">
                Replace the key placeholder with your existing Lens tracing key.
              </p>
            )}
          </>
        )}
      </section>
      {hasKey && validEndpoint && (
        <>
          {!native && (
            <CodingAgentSetup
              proxyUrl={traceUrl}
              traceUrl={traceUrl}
              instructions={instructions}
              onCopied={() => setCopiedInstructions(true)}
            />
          )}
          <div className="rounded-xl border bg-muted/20 p-5" role="status">
            <h3 className="text-sm font-medium">
              {native || copiedInstructions ? "Run one task in your app" : "Let your coding agent connect the project"}
            </h3>
            <p className="mt-2 text-sm leading-6 text-muted-foreground">
              {native || copiedInstructions
                ? "Keep this page open. Lens checks automatically and will show your first trace as soon as it arrives."
                : "Paste the setup instructions into Claude Code or Codex, then run one real task. No changes to your model provider are needed."}
            </p>
          </div>
        </>
      )}
    </div>
  );
}

function validTraceEndpoint(endpoint: string, https: boolean): boolean {
  try {
    const url = new URL(endpoint);
    return (
      (https ? url.protocol === "https:" : ["http:", "https:"].includes(url.protocol)) &&
      url.pathname.endsWith("/v1/traces") &&
      !url.username &&
      !url.password &&
      !url.search &&
      !url.hash
    );
  } catch {
    return false;
  }
}
