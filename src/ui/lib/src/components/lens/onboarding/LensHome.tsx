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
  createTracingKey,
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
  const [preparing, setPreparing] = useState(false);
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
  const canCreate = canMintTracingKey && !readOnly;
  const usingExisting =
    !tracingKey && (progress.existing || Boolean(progress.keyId));
  const rememberKey = (
    value: string,
    _active: boolean,
    record: TracingKeyRecord,
  ) => {
    setCreated({ value, record });
    setProgress({ keyId: record.id, startedAt: null, existing: false });
    cache.setQueryData<TracingKeyRecord[]>(
      tracingKeysQuery(accessToken),
      (current) => [
        ...(current ?? []).filter((key) => key.id !== record.id),
        record,
      ],
    );
    void keys.refetch();
  };
  const prepareKey = async () => {
    setPreparing(true);
    try {
      const result = await createTracingKey(
        accessToken,
        project?.name ?? "Agent tracing",
      );
      rememberKey(result.key, result.active, result.record);
      return result.key;
    } finally {
      setPreparing(false);
    }
  };
  const environment = (key?: string) =>
    `export LITELLM_TRACING_KEY=${shellValue(key ?? "<your tracing key>")}\nexport OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=${shellValue(`${traceUrl}/v1/traces`)}\nexport OTEL_EXPORTER_OTLP_TRACES_HEADERS="Authorization=Bearer $LITELLM_TRACING_KEY"\nexport OTEL_EXPORTER_OTLP_PROTOCOL="http/protobuf"`;

  const keyChoices = (
    <>
      {!canCreate && (
        <p className="text-sm text-muted-foreground">
          Ask your Lens administrator for a tracing key, or use one already in
          your project.
        </p>
      )}
      {usingExisting && !tracingKey && (
        <div className="space-y-2 text-sm">
          {selected ? (
            <>
              <p className="font-medium">Using {selected.name}</p>
              <p className="text-muted-foreground">
                Lens cannot show this key again. The instructions use the copy
                saved in your project.
              </p>
            </>
          ) : (
            <p role="status" className="text-muted-foreground">
              The instructions will use the tracing key in your project’s
              environment.
            </p>
          )}
          <div className="flex flex-wrap gap-3">
            {canViewInvestigations && (
              <Button
                variant="link"
                size="sm"
                className="h-auto px-0"
                onClick={() => setManageKeys(true)}
              >
                Choose another key
              </Button>
            )}
            {canCreate && (
              <Button
                variant="link"
                size="sm"
                className="h-auto px-0"
                onClick={restart}
              >
                Use a new key
              </Button>
            )}
          </div>
        </div>
      )}
      <div className="flex flex-wrap items-center gap-x-5 gap-y-2 border-t pt-4">
        {!usingExisting && !tracingKey && (
          <Button
            variant="link"
            size="sm"
            className="h-auto px-0"
            disabled={preparing}
            onClick={() =>
              setProgress((current) => ({ ...current, existing: true }))
            }
          >
            Use an existing key
          </Button>
        )}
        {canViewInvestigations && (
          <Button
            variant="link"
            size="sm"
            className="h-auto px-0"
            disabled={preparing}
            onClick={() => setManageKeys(true)}
          >
            Manage tracing keys
          </Button>
        )}
      </div>
    </>
  );

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
              className="lens-panel space-y-5 p-5 sm:p-6"
              aria-labelledby="home-connect-heading"
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <h3
                  id="home-connect-heading"
                  className="text-base font-semibold"
                >
                  {manual ? "Set up manually" : "Set up with your coding agent"}
                </h3>
                <Button
                  variant="link"
                  size="sm"
                  className="h-auto px-0 text-xs"
                  aria-expanded={manual}
                  disabled={preparing}
                  onClick={() => setManual((value) => !value)}
                >
                  {manual ? "Use a coding agent" : "Set up manually"}
                </Button>
              </div>
              {manual ? (
                <div className="space-y-5" id="tracing-key">
                  <div className="space-y-3">
                    <h4 className="lens-section-label">1. Get a tracing key</h4>
                    <p className="text-sm leading-6 text-muted-foreground">
                      A tracing key lets your agent send telemetry to Lens. Keep
                      your existing model API key for model calls.
                    </p>
                    {ready && canCreate && !usingExisting && (
                      <TracingKey
                        accessToken={accessToken}
                        tracingKey={tracingKey ?? null}
                        name={project?.name ?? "Agent tracing"}
                        compact
                        primary
                        onCreated={rememberKey}
                      />
                    )}
                    {keyChoices}
                  </div>
                  <div className="space-y-3">
                    <h4 className="lens-section-label">
                      2. Configure your project
                    </h4>
                    <p className="text-sm leading-6 text-muted-foreground">
                      Add these variables where your project loads its
                      environment. Configure your framework’s OpenTelemetry
                      exporter and include
                      <code> gen_ai.agent.name</code> on its root agent span.
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
                        Restore the Lens connection to get your tracing
                        endpoint.
                      </p>
                    )}
                    <p className="text-xs leading-5 text-muted-foreground">
                      For Moyai, use <code>LITELLM_TRACE_ENDPOINT</code> with an
                      HTTPS URL ending in <code>/v1/traces</code> and save the
                      key as <code>LITELLM_TRACE_API_KEY</code>. Its tracing is
                      built in.
                    </p>
                  </div>
                </div>
              ) : (
                <SetupAgentPrompt
                  goal="tracing"
                  connection={connection.error ? undefined : connection.data}
                  agentName={project?.name}
                  tracingKey={tracingKey}
                  prepareTracingKey={
                    ready && canCreate && !usingExisting && !tracingKey
                      ? prepareKey
                      : undefined
                  }
                  prominent
                  onCopied={startWatching}
                />
              )}
              {!manual && keyChoices}
              {tracingKey && !manual && (
                <details className="text-sm" id="tracing-key">
                  <summary className="w-fit cursor-pointer text-muted-foreground">
                    Your tracing key
                  </summary>
                  <div className="mt-3">
                    <CodeBlock
                      code={tracingKey}
                      display={maskSecret(tracingKey)}
                      copyLabel="Copy tracing key"
                    />
                    <p className="mt-2 text-xs leading-5 text-muted-foreground">
                      This key only sends telemetry and checks delivery. Keep
                      your model API key for model calls. Save it before
                      refreshing; Lens cannot show it again.
                    </p>
                  </div>
                </details>
              )}
            </section>
            <section
              className="lens-panel space-y-4 p-5 sm:p-6"
              aria-labelledby="home-trace-heading"
            >
              <h3 id="home-trace-heading" className="lens-section-label">
                See your trace
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
