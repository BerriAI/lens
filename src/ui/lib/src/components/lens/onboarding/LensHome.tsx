"use client";

import { useState } from "react";
import { ArrowRight } from "lucide-react";
import { Button } from "../../ui/button";
import type { LensAgents } from "../agents/AgentScoped";
import { useLensAccessToken } from "../data/LensServices";
import { useConnectProjectRoute } from "../route";
import { useOnboarding } from "./OnboardingContext";
import { HomeConnectionStatus } from "./HomeConnectionStatus";
import { SetupAgentPrompt } from "./SetupAgentPrompt";
import {
  CodeBlock,
  maskSecret,
  TracingKey,
  useLensService,
} from "./tracing/TracingSetupCard";

const shellValue = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

export function LensHome({
  agents,
  enabled,
  onOpenAgent,
  onOpenAgents,
  onSetup,
  onConnectGitHub,
}: {
  readonly agents: LensAgents;
  readonly enabled: boolean;
  readonly onOpenAgent: (name: string) => void;
  readonly onOpenAgents: () => void;
  readonly onSetup?: () => void;
  readonly onConnectGitHub?: (name: string) => void;
}) {
  const [project] = useConnectProjectRoute();
  const [discoverAfter] = useState(Date.now);
  const accessToken = useLensAccessToken();
  const connection = useLensService(accessToken, { enabled });
  const { canMintTracingKey, readOnly } = useOnboarding();
  const [tracingKey, setTracingKey] = useState<string | null>(null);
  const [keyReady, setKeyReady] = useState<boolean | null>(null);
  const traceUrl = connection.data?.url?.replace(/\/$/, "") ?? "";
  const ready = Boolean(
    connection.data?.connected &&
      connection.data.status.storage_ready &&
      traceUrl,
  );
  const environment = (key: string | null) =>
    `export LITELLM_TRACING_KEY=${shellValue(key ?? "<your tracing key>")}\nexport OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=${shellValue(`${traceUrl}/v1/traces`)}\nexport OTEL_EXPORTER_OTLP_TRACES_HEADERS="Authorization=Bearer $LITELLM_TRACING_KEY"\nexport OTEL_EXPORTER_OTLP_PROTOCOL="http/protobuf"`;

  return (
    <section
      aria-label="Get started with Lens"
      className="mx-auto w-full max-w-5xl px-4 py-6 sm:px-6 sm:py-8"
    >
      <div className="mb-6 flex flex-wrap items-start justify-between gap-4">
        <div className="space-y-2">
          <h2 className="text-xl font-medium tracking-tight">
            Get your first trace
          </h2>
          <p className="text-sm text-muted-foreground">
            Let your coding agent connect your project. Lens will watch for its
            trace here.
          </p>
        </div>
        {agents.list.agents.length > 0 && (
          <Button variant="outline" size="sm" onClick={onOpenAgents}>
            View agents <ArrowRight aria-hidden className="size-3.5" />
          </Button>
        )}
      </div>
      {!enabled ? (
        <p className="text-sm text-muted-foreground">
          Turn off demo data to connect your project.
        </p>
      ) : (
        <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_280px]">
          <div className="min-w-0 space-y-5">
            <SetupAgentPrompt
              goal="tracing"
              connection={connection.data}
              agentName={project?.name}
              prominent
            />
            <details
              className="rounded-md border bg-card px-5 py-4"
              id="tracing-key"
            >
              <summary className="cursor-pointer text-sm font-medium">
                Tracing key
              </summary>
              <div className="mt-4 space-y-4">
                <p className="text-sm leading-6 text-muted-foreground">
                  Your coding agent will use the key already in your project. If
                  it needs one, create a dedicated key here and save it in your
                  local environment as <code>LITELLM_TRACING_KEY</code>. Keep it
                  out of the prompt.
                </p>
                {ready && canMintTracingKey && !readOnly ? (
                  <TracingKey
                    accessToken={accessToken}
                    tracingKey={tracingKey}
                    compact
                    name={project?.name ?? "Agent tracing"}
                    onCreated={(key, active) => {
                      setTracingKey(key);
                      setKeyReady(active);
                    }}
                  />
                ) : (
                  <p className="text-sm text-muted-foreground">
                    {!canMintTracingKey || readOnly
                      ? "Ask your Lens administrator for a tracing key."
                      : "A working Lens connection and trace storage are needed to create a tracing key."}
                  </p>
                )}
              </div>
            </details>
            {ready && (
              <details className="rounded-md border bg-card px-5 py-4">
                <summary className="cursor-pointer text-sm font-medium">
                  Set up manually
                </summary>
                <p className="my-4 text-sm leading-6 text-muted-foreground">
                  Add these variables where your project loads its environment.
                  Keep its existing agent name and model settings.
                </p>
                <CodeBlock
                  code={environment(tracingKey)}
                  display={environment(tracingKey && maskSecret(tracingKey))}
                  copyLabel="Copy project environment"
                  wrap
                />
                <p className="mt-3 text-xs leading-5 text-muted-foreground">
                  For Moyai, use <code>LITELLM_TRACE_ENDPOINT</code> with an
                  HTTPS URL ending in <code>/v1/traces</code> and save the key
                  as <code>LITELLM_TRACE_API_KEY</code>. Its tracing is built
                  in.
                </p>
              </details>
            )}
          </div>
          <HomeConnectionStatus
            name={project?.name ?? ""}
            discoverAfter={discoverAfter}
            enabled={enabled}
            keyReady={keyReady}
            onOpenTraces={onOpenAgent}
            onSetup={onSetup}
            onConnectGitHub={onConnectGitHub}
          />
        </div>
      )}
      {onSetup && (
        <Button
          variant="link"
          size="sm"
          className="mt-6 px-0 text-xs text-muted-foreground"
          onClick={onSetup}
        >
          Deployment setup
        </Button>
      )}
    </section>
  );
}
