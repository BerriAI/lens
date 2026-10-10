"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useInterval } from "usehooks-ts";
import { ArrowRight, Check, Loader2 } from "lucide-react";
import { Button } from "../../ui/button";
import { useLensAccessToken } from "../data/LensServices";
import { useTracesApi } from "../traces/api";
import { useOnboarding } from "./OnboardingContext";
import { connectionAuthStatus } from "./connectionErrors";
import { findConnectionTrace } from "./homeConnection";
import type { TracingKeyRecord } from "./tracing/tracingKeys";

export function HomeConnectionStatus({
  keyRecord,
  startedAt,
  enabled,
  onRestart,
  onSelectKey,
}: {
  readonly keyRecord?: TracingKeyRecord;
  readonly startedAt: number | null;
  readonly enabled: boolean;
  readonly onRestart: () => void;
  readonly onSelectKey?: () => void;
}) {
  const accessToken = useLensAccessToken();
  const traces = useTracesApi(accessToken);
  const { openTrace } = useOnboarding();
  const [clock, setClock] = useState(Date.now);
  const [retryAt, setRetryAt] = useState(0);
  const waiting =
    enabled && Boolean(keyRecord) && startedAt !== null && traces.live;
  const expired = clock - Math.max(startedAt ?? clock, retryAt) >= 5 * 60_000;
  const receipt = useQuery({
    queryKey: ["lens-home-receipt", accessToken, keyRecord?.id, startedAt],
    queryFn: ({ signal }) =>
      findConnectionTrace(
        accessToken,
        traces,
        keyRecord!.tenant.api_key_hash,
        startedAt!,
        signal,
      ),
    enabled: waiting && !expired,
    retry: false,
    refetchInterval: (query) =>
      waiting && !expired && !query.state.data && !query.state.error
        ? 3000
        : false,
    refetchOnWindowFocus: false,
  });
  useInterval(
    () => setClock(Date.now()),
    waiting && !receipt.data && !receipt.error && !expired ? 1000 : null,
  );
  const matched = waiting ? receipt.data : null;
  const auth = connectionAuthStatus(receipt.error);
  return (
    <div className="space-y-3" aria-label="Trace delivery" role="region">
      <div
        className="flex items-center gap-2 text-sm font-medium"
        role="status"
      >
        {matched ? (
          <Check aria-hidden className="size-4 text-success" />
        ) : waiting && !expired && !receipt.error ? (
          <Loader2
            aria-hidden
            className="size-4 motion-safe:animate-spin text-muted-foreground"
          />
        ) : null}
        {matched
          ? "Trace received"
          : receipt.error
            ? "Could not check for your trace"
            : expired
              ? "No trace received yet"
              : startedAt !== null
                ? "Waiting for your agent’s trace"
                : "Run one task"}
      </div>
      <p className="text-sm leading-6 text-muted-foreground">
        {matched
          ? `A new trace from ${matched.agent_names?.[0] || matched.name || matched.service} arrived using “${keyRecord?.name}”. Open it to see the agent’s work.`
          : auth === 401
            ? "Sign in again to check delivery."
            : auth === 403
              ? "Your account cannot check delivery. Ask your Lens administrator for access."
              : receipt.error
                ? "We couldn’t check delivery. Your setup is saved; retry the check."
                : expired
                  ? "Check the tracing endpoint and key, restart your project, then run a task. Your setup is saved."
                  : "Run a real task in your project. Your coding agent will verify that exact run and give you a link to its trace."}
      </p>
      {matched ? (
        <div className="flex flex-wrap gap-2">
          <Button onClick={() => openTrace(matched)}>
            Open trace <ArrowRight aria-hidden className="size-4" />
          </Button>
          <Button variant="outline" onClick={onRestart}>
            Connect another agent
          </Button>
        </div>
      ) : auth === 401 ? (
        <Button variant="outline" onClick={() => window.location.reload()}>
          Reload to sign in
        </Button>
      ) : waiting && (receipt.error || expired) && !auth ? (
        <Button
          variant="outline"
          disabled={receipt.isFetching}
          onClick={() => {
            setClock(Date.now());
            setRetryAt(Date.now());
            void receipt.refetch();
          }}
        >
          Check again
        </Button>
      ) : !keyRecord && startedAt !== null && onSelectKey ? (
        <Button variant="link" className="h-auto px-0" onClick={onSelectKey}>
          Select an existing tracing key
        </Button>
      ) : null}
    </div>
  );
}
