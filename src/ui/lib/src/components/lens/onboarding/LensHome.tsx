"use client";

import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useSessionStorage } from "usehooks-ts";
import { ArrowRight } from "lucide-react";
import { Button } from "../../ui/button";
import type { LensAgents } from "../agents/AgentScoped";
import { useLensAccessToken } from "../data/LensServices";
import { useConnectProjectRoute } from "../route";
import { useOnboarding } from "./OnboardingContext";
import { HomeConnectionStatus } from "./HomeConnectionStatus";
import { SetupAgentPrompt } from "./SetupAgentPrompt";
import { LensPageHeader } from "../ui/LensPageHeader";
import {
  CodeBlock,
  TracingKey,
  useLensService,
} from "./tracing/TracingSetupCard";
import {
  maskSecret,
  tracingKeysQuery,
  useTracingKeys,
  type TracingKeyRecord,
} from "./tracing/tracingKeys";
import { TracingKeysDialog } from "./tracing/TracingKeysDialog";
import { emptyConnection, restoreConnection } from "./homeConnection";
import { connectionAuthStatus } from "./connectionErrors";

const shellValue = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

export function LensHome({
  agents,
  enabled,
  onOpenAgents,
  onSetup,
}: {
  readonly agents: LensAgents;
  readonly enabled: boolean;
  readonly onOpenAgents: () => void;
  readonly onSetup?: () => void;
}) {
  const [project] = useConnectProjectRoute();
  const accessToken = useLensAccessToken();
  const connection = useLensService(accessToken, {
    enabled,
    refetchInterval: 3000,
  });
  const { canMintTracingKey, canViewInvestigations, readOnly } =
    useOnboarding();
  const keys = useTracingKeys(accessToken, enabled && canViewInvestigations);
  const cache = useQueryClient();
  const [progress, setProgress] = useSessionStorage(
    "lens.agent-connection",
    emptyConnection,
    {
      initializeWithValue: false,
      deserializer: restoreConnection,
    },
  );
  const [created, setCreated] = useState<{
    value: string;
    record: TracingKeyRecord;
  } | null>(null);
  const [manageKeys, setManageKeys] = useState(false);
  const [manual, setManual] = useState(false);
  const selected = progress.keyId
    ? keys.data?.find((key) => key.id === progress.keyId) ??
      (!keys.isSuccess && created?.record.id === progress.keyId
        ? created.record
        : undefined)
    : undefined;
  const tracingKey =
    created?.record.id === selected?.id ? created?.value : undefined;
  const traceUrl = connection.data?.url?.replace(/\/$/, "") ?? "";
  const ready = Boolean(
    connection.data?.connected &&
      connection.data.status.storage_ready &&
      traceUrl,
  );
  const auth = connectionAuthStatus(connection.error);
  const issue =
    auth === 401
      ? "Sign in again to connect your agent."
      : auth === 403
        ? "Ask your Lens administrator for access to this connection."
        : connection.error
          ? "Could not reach Lens. Retry the connection check."
          : !connection.data
            ? "Checking your Lens connection…"
            : !connection.data.connected
              ? "Lens is unavailable. Check your deployment, then retry."
              : !connection.data.status.storage_ready
                ? "Trace storage is not ready. Check your deployment, then retry."
                : !traceUrl
                  ? "Set your public Lens address in deployment setup so your agent knows where to send traces."
                  : connection.data.status.credentials_ready === false
                    ? "Lens is syncing tracing credentials. This page will update when they are ready."
                    : null;
  const startWatching = () =>
    setProgress((current) => ({
      ...current,
      startedAt: current.startedAt ?? Date.now(),
    }));
  const restart = () => {
    setCreated(null);
    setProgress(emptyConnection);
  };
  const environment = (key?: string) =>
    `export LITELLM_TRACING_KEY=${shellValue(key ?? "<your tracing key>")}\nexport OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=${shellValue(`${traceUrl}/v1/traces`)}\nexport OTEL_EXPORTER_OTLP_TRACES_HEADERS="Authorization=Bearer $LITELLM_TRACING_KEY"\nexport OTEL_EXPORTER_OTLP_PROTOCOL="http/protobuf"`;

  return (
    <section aria-label="Connect an agent" className="w-full">
      <LensPageHeader
        page="home"
        title="Connect a new agent"
        description="Send a trace from your project and see what your agent is doing."
        actions={
          agents.list.agents.length > 0 && (
            <Button variant="outline" size="sm" onClick={onOpenAgents}>
              View agents <ArrowRight aria-hidden className="size-3.5" />
            </Button>
          )
        }
      />
      <div className="lens-page-body max-w-3xl space-y-6">
        {!enabled ? (
          <p className="text-sm text-muted-foreground">
            Turn off demo data to connect your project.
          </p>
        ) : (
          <>
            {issue && (
              <div
                role="status"
                className="rounded-lg border bg-muted/30 p-4 text-sm"
              >
                <p>{issue}</p>
                {connection.data || connection.error ? (
                  <div className="mt-3 flex flex-wrap gap-2">
                    {auth === 401 ? (
                      <Button
                        size="sm"
                        onClick={() => window.location.reload()}
                      >
                        Reload to sign in
                      </Button>
                    ) : (
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={connection.isFetching}
                        onClick={() => void connection.refetch()}
                      >
                        Retry connection
                      </Button>
                    )}
                    {onSetup && !auth && (
                      <Button size="sm" variant="outline" onClick={onSetup}>
                        Deployment setup
                      </Button>
                    )}
                  </div>
                ) : null}
              </div>
            )}
            <section
              className="lens-panel space-y-4 p-5 sm:p-6"
              aria-labelledby="home-key-heading"
              id="tracing-key"
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <h3 id="home-key-heading" className="lens-section-label">
                  1. Get a tracing key
                </h3>
                {canViewInvestigations && (
                  <Button
                    variant="link"
                    size="sm"
                    className="h-auto px-0 text-xs"
                    onClick={() => setManageKeys(true)}
                  >
                    Manage tracing keys
                  </Button>
                )}
              </div>
              <p className="text-sm leading-6 text-muted-foreground">
                A tracing key lets your agent send telemetry to Lens. Keep using
                your existing model API key for model calls.
              </p>
              {selected && !tracingKey ? (
                <div className="space-y-2">
                  <p className="text-sm font-medium">Using {selected.name}</p>
                  <p className="text-sm text-muted-foreground">
                    Lens cannot show this key again. Use the copy saved in your
                    project
                    {canMintTracingKey && !readOnly
                      ? ", or create a new key."
                      : "."}
                  </p>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={
                      canMintTracingKey && !readOnly
                        ? restart
                        : () => setManageKeys(true)
                    }
                  >
                    {canMintTracingKey && !readOnly
                      ? "Create a new key"
                      : "Choose another key"}
                  </Button>
                </div>
              ) : ready && canMintTracingKey && !readOnly ? (
                <TracingKey
                  accessToken={accessToken}
                  tracingKey={tracingKey ?? null}
                  name={project?.name ?? "Agent tracing"}
                  compact
                  primary={!progress.existing}
                  onCreated={(value, _active, record) => {
                    setCreated({ value, record });
                    setProgress({
                      keyId: record.id,
                      startedAt: null,
                      existing: false,
                    });
                    cache.setQueryData<TracingKeyRecord[]>(
                      tracingKeysQuery(accessToken),
                      (current) => [...(current ?? []), record],
                    );
                    void keys.refetch();
                  }}
                />
              ) : (
                <p className="text-sm text-muted-foreground">
                  {!canMintTracingKey || readOnly
                    ? "Ask your Lens administrator for a tracing key, or use one already in your project."
                    : "A working Lens connection and trace storage are needed to create a tracing key."}
                </p>
              )}
              {!selected && (
                <Button
                  variant="link"
                  size="sm"
                  className="h-auto px-0"
                  onClick={() =>
                    setProgress((current) => ({ ...current, existing: true }))
                  }
                >
                  My project already has one
                </Button>
              )}
              {progress.existing && !selected && (
                <p role="status" className="text-sm text-muted-foreground">
                  The setup instructions will use the key in your project’s
                  environment.
                  {canViewInvestigations && (
                    <>
                      {" "}
                      <Button
                        variant="link"
                        size="sm"
                        className="h-auto px-0"
                        onClick={() => setManageKeys(true)}
                      >
                        Select an existing key
                      </Button>
                    </>
                  )}
                </p>
              )}
            </section>
            <section
              className="lens-panel space-y-4 p-5 sm:p-6"
              aria-labelledby="home-connect-heading"
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <h3 id="home-connect-heading" className="lens-section-label">
                  2. Connect your agent
                </h3>
                <Button
                  variant="link"
                  size="sm"
                  className="h-auto px-0 text-xs"
                  aria-expanded={manual}
                  onClick={() => setManual((value) => !value)}
                >
                  {manual ? "Use a coding agent" : "Set up manually"}
                </Button>
              </div>
              {manual ? (
                <div className="space-y-4">
                  <p className="text-sm leading-6 text-muted-foreground">
                    Add these variables where your project loads its
                    environment. Configure your framework’s OpenTelemetry
                    exporter and include <code>gen_ai.agent.name</code> on its
                    root agent span.
                  </p>
                  {ready ? (
                    <CodeBlock
                      code={environment(tracingKey)}
                      display={environment(
                        tracingKey ? maskSecret(tracingKey) : undefined,
                      )}
                      copyLabel="Copy project environment"
                      wrap
                      onCopied={startWatching}
                    />
                  ) : (
                    <p className="text-sm text-muted-foreground">
                      Restore the Lens connection to get your tracing endpoint.
                    </p>
                  )}
                  <p className="text-xs leading-5 text-muted-foreground">
                    For Moyai, use <code>LITELLM_TRACE_ENDPOINT</code> with an
                    HTTPS URL ending in <code>/v1/traces</code> and save the key
                    as <code>LITELLM_TRACE_API_KEY</code>. Its tracing is built
                    in.
                  </p>
                </div>
              ) : (
                <SetupAgentPrompt
                  goal="tracing"
                  connection={connection.error ? undefined : connection.data}
                  agentName={project?.name}
                  tracingKey={tracingKey}
                  prominent
                  primaryAction={Boolean(
                    (selected || progress.existing) &&
                      progress.startedAt === null,
                  )}
                  onCopied={startWatching}
                />
              )}
            </section>
            <section
              className="lens-panel space-y-4 p-5 sm:p-6"
              aria-labelledby="home-trace-heading"
            >
              <h3 id="home-trace-heading" className="lens-section-label">
                3. See your trace
              </h3>
              <HomeConnectionStatus
                keyRecord={selected}
                startedAt={progress.startedAt}
                enabled={enabled && ready && !issue}
                onRestart={restart}
                onSelectKey={
                  canViewInvestigations ? () => setManageKeys(true) : undefined
                }
              />
            </section>
            {canViewInvestigations && (
              <TracingKeysDialog
                open={manageKeys}
                onOpenChange={setManageKeys}
                onSelect={(record) => {
                  setCreated((current) =>
                    current?.record.id === record.id ? current : null,
                  );
                  setProgress((current) => ({
                    keyId: record.id,
                    startedAt:
                      current.keyId === record.id ? current.startedAt : null,
                    existing: true,
                  }));
                }}
                onRevoked={(id) => {
                  if (id === progress.keyId) restart();
                }}
              />
            )}
          </>
        )}
      </div>
    </section>
  );
}
