"use client";

import { useState } from "react";
import { Activity, ArrowUpRight, RefreshCw } from "lucide-react";
import { STANDALONE_DOCS_URL, useLensHost } from "../../../host/LensHost";
import { Button } from "../../ui/button";
import { CodingAgentSetup, Endpoints, TracingKey, useLensService } from "./tracing/TracingSetupCard";

export function WaitingForTraces({
  onConnect,
  onCheck,
  checking,
  detail,
  accessToken,
  canMintTracingKey,
  readOnly,
}: {
  readonly onConnect?: () => void;
  readonly onCheck: () => void;
  readonly checking: boolean;
  readonly detail?: string | null;
  readonly accessToken: string;
  readonly canMintTracingKey: boolean;
  readonly readOnly: boolean;
}) {
  const standalone = useLensHost().surface === "standalone";
  const connection = useLensService(accessToken);
  const [tracingKey, setTracingKey] = useState<string | null>(null);
  const traceUrl = connection.data?.url?.replace(/\/$/, "");
  return (
    <section
      aria-label="Waiting for traces"
      className="my-5 flex min-h-96 flex-1 flex-col items-center rounded-xl border bg-muted/20 px-5 py-12 sm:py-20"
    >
      <div className="w-full max-w-2xl">
        <span
          role="status"
          className="inline-flex items-center gap-2 rounded-full border bg-background px-3 py-1.5 text-xs font-medium text-muted-foreground"
        >
          <Activity aria-hidden className="size-3.5" />
          Waiting for traces
        </span>
        <h2 className="mt-5 text-xl font-semibold tracking-tight">See what your agent is doing</h2>
        <p className="mt-2 text-sm leading-6 text-muted-foreground">
          Connect your project to Lens, then run a task. Your traces will appear here.
        </p>
        {traceUrl ? (
          <>
            <CodingAgentSetup proxyUrl={traceUrl} traceUrl={traceUrl} />
            <details className="mt-5 border-t pt-4">
              <summary className="cursor-pointer text-sm font-medium">Connection details</summary>
              <div className="mt-4 space-y-4">
                {canMintTracingKey && !readOnly ? (
                  <TracingKey accessToken={accessToken} tracingKey={tracingKey} onCreated={setTracingKey} />
                ) : (
                  <p className="text-sm text-muted-foreground">
                    Ask your Lens administrator for a dedicated tracing key.
                  </p>
                )}
                <Endpoints proxyUrl={traceUrl} />
              </div>
            </details>
          </>
        ) : (
          <div className="mt-7 rounded-lg border bg-card p-5 sm:p-6">
            <div className="flex items-center justify-between gap-4">
              <h3 className="text-sm font-medium">Start tracing your agent</h3>
              <a
                href={standalone ? STANDALONE_DOCS_URL : "https://docs.litellm.ai/docs/proxy/lens"}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
              >
                Docs
                <ArrowUpRight aria-hidden className="size-3.5" />
              </a>
            </div>
            <p className="mt-2 text-sm leading-6 text-muted-foreground">
              {onConnect
                ? "Get the endpoint, tracing key, and setup instructions for your project."
                : "Ask your Lens administrator for the endpoint and tracing key."}
            </p>
          </div>
        )}
        {detail && (
          <p role="alert" className="mt-4 text-sm text-muted-foreground">
            Trace storage is not configured yet. Your Lens administrator can complete deployment setup in Home.
          </p>
        )}
        <div className="mt-3 flex flex-wrap justify-end gap-2">
          {onConnect && (
            <Button variant="ghost" size="sm" onClick={onConnect} className="text-xs text-muted-foreground">
              Set up manually
            </Button>
          )}
          <Button
            variant="ghost"
            size="sm"
            disabled={checking}
            onClick={onCheck}
            className="text-xs text-muted-foreground"
          >
            <RefreshCw aria-hidden className="size-3.5" />
            {checking ? "Checking…" : "Check for traces"}
          </Button>
        </div>
      </div>
    </section>
  );
}
