"use client";

import { useEffect, useRef, useState } from "react";
import { ArrowRight, ArrowUpRight, Pencil } from "lucide-react";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../ui/select";
import type { LensAgents } from "../agents/AgentScoped";
import { useLensAccessToken } from "../data/LensServices";
import { useConnectProjectRoute, useProjectSetupRoute, type ProjectConnection } from "../route";
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
  const setup = useProjectSetupRoute();
  const [name, setName] = useState(project?.name ?? "");
  const [integration, setIntegration] = useState<ProjectConnection["integration"]>(project?.integration ?? "auto");
  const [keyStatus, setKeyStatus] = useState<(ProjectConnection & { readonly active: boolean }) | null>(null);
  const [configurationIssue, setConfigurationIssue] = useState<string | null>(null);
  const keyReady =
    keyStatus?.name === project?.name && keyStatus?.integration === project?.integration ? keyStatus?.active : null;
  const nameInput = useRef<HTMLInputElement>(null);
  const projectTitle = useRef<HTMLHeadingElement>(null);
  const moveFocus = useRef(false);
  useEffect(() => {
    if (!moveFocus.current) return;
    (project ? projectTitle.current : nameInput.current)?.focus();
    moveFocus.current = false;
  }, [project?.name, project?.integration]);
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
          <Button variant="outline" size="sm" className="hidden sm:inline-flex" onClick={onOpenAgents}>
            View agents
            <ArrowRight aria-hidden className="size-3.5" />
          </Button>
        )}
      </div>
      {!enabled ? (
        <p className="text-sm text-muted-foreground">Turn off demo data to connect your project.</p>
      ) : (
        <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_320px]">
          <HomeConnectionStatus
            name={project?.name ?? ""}
            enabled={enabled}
            stage={!project ? "name" : setup.showInstructions ? "instructions" : "key"}
            keyReady={keyReady}
            setupIssue={project && setup.showInstructions ? configurationIssue : null}
            onOpenTraces={onOpenAgent}
            onSetup={onSetup}
          />
          <div className="min-w-0 lg:col-start-1 lg:row-start-1">
            {project ? (
              <>
                <div className="mb-4 flex items-center justify-between gap-3 rounded-lg border bg-muted/20 px-4 py-3">
                  <div className="min-w-0">
                    <h3 ref={projectTitle} tabIndex={-1} className="truncate text-sm font-medium outline-none">
                      {project.name}
                    </h3>
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
                      moveFocus.current = true;
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
                  setup={setup}
                  onConfigurationIssue={setConfigurationIssue}
                  onKeyReady={(active) => setKeyStatus({ ...project, active })}
                />
              </>
            ) : (
              <form
                className="space-y-5 rounded-xl border bg-card p-5 sm:p-6"
                onSubmit={(event) => {
                  event.preventDefault();
                  if (!name.trim()) return;
                  moveFocus.current = true;
                  setProject({ name: name.trim(), integration });
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
                    ref={nameInput}
                    name="agent-name"
                    autoComplete="off"
                    autoCapitalize="none"
                    spellCheck={false}
                    aria-describedby="home-agent-name-help"
                    placeholder="e.g. moyai"
                    value={name}
                    onChange={(event) => setName(event.target.value)}
                    maxLength={128}
                    required
                  />
                  <p id="home-agent-name-help" className="text-xs text-muted-foreground">
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
  setup,
  onConfigurationIssue,
  onKeyReady,
}: {
  readonly project: ProjectConnection;
  readonly setup: ReturnType<typeof useProjectSetupRoute>;
  readonly onConfigurationIssue: (issue: string | null) => void;
  readonly onKeyReady: (ready: boolean) => void;
}) {
  const accessToken = useLensAccessToken();
  const { canMintTracingKey, readOnly } = useOnboarding();
  const connection = useLensService(accessToken);
  const [tracingKey, setTracingKey] = useState<string | null>(null);
  const [endpointOverride, setEndpointOverride] = useState<string | null>(setup.endpoint);
  const [copiedInstructions, setCopiedInstructions] = useState(false);
  const traceUrl = connection.data?.url?.replace(/\/$/, "") ?? "";
  const endpoint = endpointOverride ?? `${traceUrl}/v1/traces`;
  const native = project.integration === "moyai";
  const ready = Boolean(connection.data?.connected && connection.data.status.storage_ready && traceUrl);
  const hasKey = tracingKey !== null || setup.showInstructions;
  const validEndpoint = validTraceEndpoint(endpoint, native);
  useEffect(() => {
    onConfigurationIssue(
      hasKey && ready && !validEndpoint
        ? native
          ? "Set an HTTPS traces endpoint in Configure Moyai before running a task."
          : "Check the public Lens address in deployment setup before running a task."
        : null,
    );
  }, [hasKey, ready, validEndpoint, native, onConfigurationIssue]);
  const environment = (secret: string | null) =>
    native
      ? `LITELLM_TRACE_ENDPOINT=${shellValue(endpoint)}\nLITELLM_TRACE_API_KEY=${shellValue(secret ?? "<your tracing key>")}`
      : `export LITELLM_TRACING_KEY=${shellValue(secret ?? "<your tracing key>")}\nexport OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=${shellValue(endpoint)}\nexport OTEL_EXPORTER_OTLP_TRACES_HEADERS="Authorization=Bearer $LITELLM_TRACING_KEY"\nexport OTEL_EXPORTER_OTLP_PROTOCOL="http/protobuf"`;
  const instructions = `${projectSetupPrompt(traceUrl)}\n\nThe agent name to look for in Lens is ${JSON.stringify(project.name)}. For new instrumentation, use this name in gen_ai.agent.name. If an already-instrumented app uses a different name, report it so I can select that name in Lens.`;
  const keyHeading = useRef<HTMLHeadingElement>(null);
  const focusKeyResult = useRef(false);
  useEffect(() => {
    if (!focusKeyResult.current) return;
    keyHeading.current?.focus();
    focusKeyResult.current = false;
  }, [hasKey]);
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
      <section
        className={
          hasKey && !tracingKey
            ? "flex flex-wrap items-center justify-between gap-2 rounded-lg border bg-muted/20 px-4 py-3"
            : "space-y-4 rounded-xl border bg-card p-5 sm:p-6"
        }
        aria-label="Project credentials"
      >
        <div className="space-y-1">
          <h3 ref={keyHeading} tabIndex={-1} className="font-medium outline-none">
            {hasKey ? (tracingKey ? "Your tracing key" : "Use your saved key") : "Get a tracing key"}
          </h3>
          <p className={hasKey && !tracingKey ? "sr-only" : "text-sm text-muted-foreground"}>
            {hasKey
              ? tracingKey
                ? `Save this key as ${native ? "LITELLM_TRACE_API_KEY" : "LITELLM_TRACING_KEY"} in your project’s local environment. Keep it out of your coding agent’s prompt.`
                : "Use the dedicated Lens tracing key you saved in your project’s environment."
              : "A dedicated key lets your project send traces to Lens."}
          </p>
        </div>
        {(!hasKey || tracingKey !== null) && canMintTracingKey && !readOnly && (
          <TracingKey
            accessToken={accessToken}
            tracingKey={tracingKey}
            compact
            name={project.name}
            onCreated={(key, active) => {
              focusKeyResult.current = true;
              setTracingKey(key);
              setup.setInstructions(true);
              onKeyReady(active);
            }}
          />
        )}
        {!hasKey && (
          <>
            {(!canMintTracingKey || readOnly) && (
              <p className="text-sm text-muted-foreground">Ask your Lens administrator for a tracing key.</p>
            )}
            <Button
              variant="link"
              size="sm"
              className="px-0"
              onClick={() => {
                focusKeyResult.current = true;
                setup.setInstructions(true);
              }}
            >
              I already have a tracing key
            </Button>
          </>
        )}
        {hasKey && !tracingKey && canMintTracingKey && !readOnly && (
          <Button
            variant="link"
            size="sm"
            className="h-auto px-0 text-xs text-muted-foreground"
            onClick={() => {
              focusKeyResult.current = true;
              setup.setInstructions(false);
            }}
          >
            Need a new tracing key?
          </Button>
        )}
      </section>
      {hasKey && (
        <>
          {!native && validEndpoint && (
            <CodingAgentSetup
              proxyUrl={traceUrl}
              traceUrl={traceUrl}
              heading="Connect with your coding agent"
              instructions={instructions}
              onCopied={() => setCopiedInstructions(true)}
            />
          )}
          {native ? (
            <section className="space-y-4 rounded-xl border bg-card p-5 sm:p-6" aria-label="Configure Moyai">
              <div className="space-y-1">
                <h3 className="font-medium">Configure Moyai</h3>
                <p className="text-sm text-muted-foreground">
                  Tracing is built in. Add these values to its environment.
                </p>
              </div>
              <div className="space-y-2">
                <label htmlFor="home-trace-endpoint" className="text-sm font-medium">
                  HTTPS traces endpoint
                </label>
                <Input
                  id="home-trace-endpoint"
                  type="url"
                  value={endpoint}
                  name="traces-endpoint"
                  autoComplete="off"
                  spellCheck={false}
                  aria-invalid={!validEndpoint}
                  aria-describedby={!validEndpoint ? "home-endpoint-error" : undefined}
                  onChange={(event) => {
                    const next = event.target.value.trim();
                    setEndpointOverride(next);
                    setup.setEndpoint(validTraceEndpoint(next, true) ? next : null);
                  }}
                />
              </div>
              {validEndpoint ? (
                <CodeBlock
                  code={environment(tracingKey)}
                  display={environment(tracingKey && maskSecret(tracingKey))}
                  copyLabel="Copy project environment"
                  wrap
                />
              ) : (
                <div className="space-y-2 text-sm" id="home-endpoint-error" role="alert">
                  <p className="text-destructive">
                    Moyai requires an HTTPS endpoint ending in /v1/traces. Put Lens behind a TLS reverse proxy, then use
                    its public HTTPS address here.
                  </p>
                  <a
                    className="inline-flex items-center gap-1 underline underline-offset-4"
                    href="https://github.com/BerriAI/lens/blob/main/deploy/lens/README.md"
                    target="_blank"
                    rel="noreferrer"
                  >
                    HTTPS deployment guide <ArrowUpRight aria-hidden className="size-3.5" />
                  </a>
                </div>
              )}
              {!tracingKey && validEndpoint && (
                <p className="text-xs text-muted-foreground">
                  Replace the key placeholder with your existing Lens tracing key.
                </p>
              )}
            </section>
          ) : validEndpoint ? (
            <details className="rounded-lg border bg-muted/20 px-4 py-3">
              <summary className="cursor-pointer text-sm font-medium">Set up manually</summary>
              <p className="my-3 text-sm text-muted-foreground">Set these variables where your agent runs.</p>
              <CodeBlock
                code={environment(tracingKey)}
                display={environment(tracingKey && maskSecret(tracingKey))}
                copyLabel="Copy project environment"
                wrap
              />
              {!tracingKey && (
                <p className="mt-3 text-xs text-muted-foreground">
                  Replace the key placeholder with your Lens tracing key.
                </p>
              )}
            </details>
          ) : (
            <p role="alert" className="text-sm text-destructive">
              Check the public Lens address in deployment setup. Traces need an HTTP or HTTPS endpoint ending in
              /v1/traces.
            </p>
          )}
          {validEndpoint && (
            <p className="text-sm leading-6 text-muted-foreground" role="status">
              {native
                ? "Restart Moyai, then run one task. Keep this page open to confirm its traces arrive."
                : copiedInstructions
                  ? "Instructions copied. Paste them in your project, then run one task. Lens will confirm delivery here."
                  : "After setup, run one task in your app. Lens will confirm delivery here."}
            </p>
          )}
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
