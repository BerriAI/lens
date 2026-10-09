"use client";

import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, ArrowRight, Check, Loader2 } from "lucide-react";
import { Button } from "../../ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../../ui/dialog";
import { Input } from "../../ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../ui/select";
import { useLensAccessToken } from "../data/LensServices";
import { useOnboarding } from "../onboarding/OnboardingContext";
import { CodeBlock, maskSecret, TracingKey, useLensService } from "../onboarding/tracing/TracingSetupCard";
import { FRAMEWORKS, standaloneFrameworkSnippet } from "../onboarding/tracing/tracingSetupGuides";
import { useTracesApi } from "../traces/api";
import { AGENT_WINDOW_DAYS } from "./useAgents";

interface AgentConnectionDialogProps {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onConnected: (agentName: string) => void;
}

export function AgentConnectionDialog({ open, onOpenChange, onConnected }: AgentConnectionDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-xl">
        {open && <AgentConnectionSetup onConnected={onConnected} />}
      </DialogContent>
    </Dialog>
  );
}

function AgentConnectionSetup({ onConnected }: Pick<AgentConnectionDialogProps, "onConnected">) {
  const [name, setName] = useState("");
  const [integration, setIntegration] = useState("otel");
  const [connecting, setConnecting] = useState(false);
  const trimmedName = name.trim();
  const guide = FRAMEWORKS.find((item) => item.id === integration);
  return (
    <>
      <DialogHeader className="pr-6">
        <DialogTitle>{connecting ? `Connect ${trimmedName}` : "Add an agent"}</DialogTitle>
        <DialogDescription>
          {connecting
            ? "Point your agent’s traces at Lens, then run it once"
            : "Give your agent a name and choose how it sends traces"}
        </DialogDescription>
      </DialogHeader>
      {connecting ? (
        <>
          <AgentConnectionInstructions name={trimmedName} integration={integration} onConnected={onConnected} />
          <Button variant="ghost" className="w-fit" onClick={() => setConnecting(false)}>
            <ArrowLeft aria-hidden="true" className="size-4" /> Back
          </Button>
        </>
      ) : (
        <form
          className="space-y-5"
          onSubmit={(event) => {
            event.preventDefault();
            if (trimmedName) setConnecting(true);
          }}
        >
          <div className="space-y-2">
            <label className="text-sm font-medium" htmlFor="new-agent-name">
              Agent name
            </label>
            <Input
              id="new-agent-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="e.g. moyai"
              maxLength={128}
              required
              autoFocus
            />
            <p className="text-xs text-muted-foreground">Use the name your agent sends with its traces</p>
          </div>
          <div className="space-y-2">
            <label className="text-sm font-medium" id="agent-integration-label">
              Integration
            </label>
            <Select
              value={integration}
              onValueChange={(value) => {
                if (!value) return;
                setIntegration(value);
                if (value === "moyai" && !trimmedName) setName("moyai");
              }}
            >
              <SelectTrigger aria-labelledby="agent-integration-label" className="w-full">
                <SelectValue>{guide?.label ?? "Moyai"}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="moyai">Moyai</SelectItem>
                {FRAMEWORKS.map((item) => (
                  <SelectItem key={item.id} value={item.id}>
                    {item.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {integration === "moyai" && (
              <p className="text-xs leading-5 text-muted-foreground">
                Moyai already includes tracing. Its default name is <code>moyai</code>; use a custom agent label here
                only if your runs already set one
              </p>
            )}
          </div>
          <p className="text-sm leading-6 text-muted-foreground">
            Your agent will appear in Agents when its first trace arrives
          </p>
          <div className="flex justify-end">
            <Button type="submit" disabled={!trimmedName}>
              Continue <ArrowRight aria-hidden="true" className="size-4" />
            </Button>
          </div>
        </form>
      )}
    </>
  );
}

const shellValue = (value: string): string => `'${value.replaceAll("'", "'\\''")}'`;

function isMoyaiEndpoint(endpoint: string): boolean {
  try {
    const url = new URL(endpoint);
    return url.protocol === "https:" && url.pathname.endsWith("/v1/traces");
  } catch {
    return false;
  }
}

function AgentConnectionInstructions({
  name,
  integration,
  onConnected,
}: {
  readonly name: string;
  readonly integration: string;
  readonly onConnected: (agentName: string) => void;
}) {
  const accessToken = useLensAccessToken();
  const { canMintTracingKey, readOnly } = useOnboarding();
  const connection = useLensService(accessToken);
  const traces = useTracesApi(accessToken);
  const queryClient = useQueryClient();
  const [key, setKey] = useState<string | null>(null);
  const [endpointOverride, setEndpointOverride] = useState<string | null>(null);
  const [receipt, setReceipt] = useState<"idle" | "checking" | "waiting" | "received" | "failed">("idle");
  const guide = FRAMEWORKS.find((item) => item.id === integration);
  const endpoint = endpointOverride ?? `${connection.data?.url?.replace(/\/$/, "") ?? ""}/v1/traces`;
  const native = integration === "moyai";
  const enabled = Boolean(connection.data?.connected && connection.data.status.storage_ready && connection.data.url);
  const httpsRequired = native && !isMoyaiEndpoint(endpoint);
  const environment = (secret: string | null) =>
    native
      ? [
          `LITELLM_TRACE_ENDPOINT=${shellValue(endpoint)}`,
          `LITELLM_TRACE_API_KEY=${shellValue(secret ?? "<your tracing key>")}`,
        ].join("\n")
      : [
          `export LITELLM_TRACING_KEY=${shellValue(secret ?? "<your tracing key>")}`,
          `export OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=${shellValue(endpoint)}`,
          'export OTEL_EXPORTER_OTLP_TRACES_HEADERS="Authorization=Bearer $LITELLM_TRACING_KEY"',
          'export OTEL_EXPORTER_OTLP_PROTOCOL="http/protobuf"',
          'export OTEL_METRICS_EXPORTER="none"',
          'export OTEL_LOGS_EXPORTER="none"',
        ].join("\n");
  const check = async () => {
    setReceipt("checking");
    const endMs = Date.now();
    try {
      const agents = await traces.agents({ startMs: endMs - AGENT_WINDOW_DAYS * 86_400_000, endMs });
      const received = agents.some((agent) => agent.name === name && agent.runs > 0);
      setReceipt(received ? "received" : "waiting");
      if (received) void queryClient.invalidateQueries({ queryKey: ["lensAgents", accessToken] });
    } catch {
      setReceipt("failed");
    }
  };
  if (connection.isPending)
    return (
      <p role="status" className="text-muted-foreground">
        Checking Lens connection…
      </p>
    );
  if (!enabled)
    return (
      <div className="space-y-3">
        <p role="alert" className="text-muted-foreground">
          Lens is not ready to receive traces. Check that Lens and its trace storage are running
        </p>
        <Button variant="outline" onClick={() => void connection.refetch()} disabled={connection.isFetching}>
          Check connection
        </Button>
      </div>
    );
  return (
    <div className="space-y-5">
      <section className="space-y-2" aria-labelledby="connection-key-heading">
        <h3 id="connection-key-heading" className="text-sm font-medium">
          1. Get a tracing key
        </h3>
        {canMintTracingKey && !readOnly ? (
          <TracingKey accessToken={accessToken} tracingKey={key} onCreated={setKey} name={name} />
        ) : (
          <p className="text-sm text-muted-foreground">Ask your Lens admin for a dedicated tracing key</p>
        )}
      </section>
      <section className="space-y-3" aria-labelledby="connection-config-heading">
        <h3 id="connection-config-heading" className="text-sm font-medium">
          2. {native ? "Add these to Moyai’s environment" : "Configure your trace exporter"}
        </h3>
        {native ? (
          <>
            <p className="text-sm text-muted-foreground">
              No tracing SDK installation is needed. Restart Moyai after saving
            </p>
            <div className="space-y-2">
              <label className="text-sm font-medium" htmlFor="agent-trace-endpoint">
                HTTPS traces endpoint
              </label>
              <Input
                id="agent-trace-endpoint"
                type="url"
                value={endpoint}
                onChange={(event) => setEndpointOverride(event.target.value)}
              />
            </div>
          </>
        ) : (
          <div className="space-y-2 text-sm text-muted-foreground">
            <p>Keep your current model provider and model credentials</p>
            <p>
              Already instrumented? Use its existing agent name. For new instrumentation, set{" "}
              <code>gen_ai.agent.name</code> on the root agent span to <code>{name}</code>, as shown in the example
              below
            </p>
          </div>
        )}
        {httpsRequired ? (
          <p role="alert" className="text-sm text-muted-foreground">
            Moyai requires an HTTPS endpoint. Enter your Lens HTTPS address including /v1/traces
          </p>
        ) : (
          <CodeBlock
            code={environment(key)}
            display={environment(key && maskSecret(key))}
            copyLabel="Copy agent configuration"
            wrap
          />
        )}
        {guide && (
          <details>
            <summary className="cursor-pointer text-sm font-medium">Instrumentation example for {guide.label}</summary>
            <div className="mt-3 space-y-3">
              {guide.install && <CodeBlock code={guide.install} copyLabel="Copy installation command" wrap />}
              <CodeBlock
                code={standaloneFrameworkSnippet(guide, connection.data?.url ?? "", name)}
                copyLabel="Copy instrumentation example"
                wrap
              />
            </div>
          </details>
        )}
      </section>
      <section className="space-y-3 border-t pt-5" aria-labelledby="connection-receipt-heading">
        <h3 id="connection-receipt-heading" className="text-sm font-medium">
          3. Run your agent
        </h3>
        <p className="text-sm text-muted-foreground">
          {receipt === "received" ? `Received traces from ${name}` : `Waiting for traces named ${name}`}
        </p>
        {receipt === "waiting" && (
          <p role="status" className="text-sm text-muted-foreground">
            No matching traces yet. Check the endpoint, key, and agent name, then run your agent again
          </p>
        )}
        {receipt === "failed" && (
          <p role="alert" className="text-sm text-destructive">
            Could not check for traces. Try again
          </p>
        )}
        {receipt === "received" ? (
          <Button onClick={() => onConnected(name)}>
            <Check aria-hidden="true" className="size-4" /> View traces
          </Button>
        ) : (
          <Button variant="outline" onClick={() => void check()} disabled={receipt === "checking" || !traces.live}>
            {receipt === "checking" && <Loader2 aria-hidden="true" className="size-4 animate-spin" />}
            {receipt === "checking" ? "Checking…" : "Check for traces"}
          </Button>
        )}
      </section>
    </div>
  );
}
