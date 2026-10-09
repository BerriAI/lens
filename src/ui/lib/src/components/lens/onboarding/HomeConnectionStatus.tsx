"use client";

import { useEffect, useId, useRef, useState } from "react";
import { Activity, ArrowRight, Check, ChevronDown, Circle, Github, RefreshCw } from "lucide-react";
import { useMediaQuery } from "usehooks-ts";
import { Button } from "../../ui/button";
import { cn } from "../../../lib/cva.config";
import { useAgentConnectionStatus } from "../agents/useAgentConnectionStatus";
import { useLensAccessToken } from "../data/LensServices";
import { useLensService } from "./tracing/TracingSetupCard";
import { connectionAuthStatus } from "./connectionErrors";

interface HomeConnectionStatusProps {
  readonly name: string;
  readonly enabled: boolean;
  readonly onOpenTraces: (name: string) => void;
  readonly onSetup?: () => void;
  readonly onConnectGitHub?: (name: string) => void;
  readonly keyReady?: boolean | null;
  readonly stage?: "name" | "key" | "instructions" | "verify";
  readonly setupIssue?: string | null;
}

export function HomeConnectionStatus({
  name,
  enabled,
  onOpenTraces,
  onSetup,
  onConnectGitHub,
  keyReady,
  stage = "instructions",
  setupIssue,
}: HomeConnectionStatusProps) {
  const [detailsOpen, setDetailsOpen] = useState(false);
  const detailsId = useId();
  const verificationHeading = useRef<HTMLHeadingElement>(null);
  const previousStage = useRef(stage);
  useEffect(() => {
    if (stage === "verify" && previousStage.current !== "verify") verificationHeading.current?.focus();
    previousStage.current = stage;
  }, [stage]);
  const desktop = useMediaQuery("(min-width: 1024px)", {
    initializeWithValue: false,
  });
  const token = useLensAccessToken();
  const service = useLensService(token, { enabled, refetchInterval: 3000 });
  const agentName = name.trim();
  const receipt = useAgentConnectionStatus(agentName, enabled);
  const authStatus = connectionAuthStatus(service.error) ?? connectionAuthStatus(receipt.error);
  const unavailable = Boolean(service.error || receipt.error);
  const apiReady = service.data?.connected ?? null;
  const storageReady = apiReady ? service.data?.status.storage_ready ?? null : null;
  const credentialsPending = service.data?.status.credentials_ready === false;
  const endpointMissing = Boolean(service.data && !service.data.url);
  const blocked =
    apiReady === false || storageReady === false || credentialsPending || endpointMissing || Boolean(setupIssue);
  const receiving = !unavailable && !blocked && receipt.status === "receiving";
  const verifying = stage === "verify";
  const checking = service.isFetching || receipt.isChecking;
  const title =
    authStatus === 401
      ? "Sign in again"
      : authStatus === 403
        ? "Access required"
        : unavailable
          ? "Unable to check"
          : blocked
            ? "Connection needs attention"
            : receiving
              ? "Receiving traces"
              : receipt.status === "waiting-for-new-traces"
                ? "Waiting for new traces"
                : "Waiting for traces";
  const guidance =
    authStatus === 401
      ? "Your session is no longer valid. Sign in again to check your connection."
      : authStatus === 403
        ? "Your account cannot check this connection. Ask your Lens administrator for access."
        : unavailable
          ? "We couldn't check the connection. Try again to see the latest status."
          : apiReady === false
            ? "Lens is unavailable. Check that the service is running, then retry."
            : storageReady === false
              ? "Trace storage isn't ready. Check your Lens deployment, then retry."
              : endpointMissing
                ? "Set your public Lens address in deployment setup so your project knows where to send traces."
                : credentialsPending
                  ? "Lens is syncing tracing credentials. Check again in a moment."
                  : setupIssue
                    ? setupIssue
                    : receiving
                      ? "A recent run is visible in Lens. Open it to inspect your agent's work."
                      : stage === "name" || !agentName
                        ? "Choose your project and agent name to watch for its traces here."
                        : stage === "key"
                          ? "Get a tracing key, then connect your project using the setup instructions."
                          : verifying
                            ? "Run your agent once. Lens will confirm a new trace here."
                            : "Finish connecting your project, then continue to verify its traces.";

  return (
    <section
      aria-label="Live connection"
      className={cn(
        "min-w-0",
        verifying
          ? "overflow-hidden rounded-md border bg-card lg:col-span-2 lg:col-start-1 lg:row-start-1 lg:grid lg:grid-cols-[minmax(0,1fr)_300px]"
          : "lg:col-start-2 lg:row-start-1 lg:border-l",
      )}
    >
      <div className={cn(verifying ? "p-5 sm:p-6" : "py-4 lg:px-5")}>
        <div className={cn("items-center justify-between gap-3", verifying ? "flex" : "hidden lg:flex")}>
          {verifying ? (
            <p className="font-mono text-[11px] text-muted-foreground">Step 3 of 3</p>
          ) : (
            <h3 className="text-sm font-medium">Live connection</h3>
          )}
          {enabled && agentName && !authStatus && (
            <span className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
              <span className="size-1.5 rounded-full bg-current motion-safe:animate-pulse" aria-hidden="true" />
              Checking every 3s
            </span>
          )}
        </div>
        {verifying && (
          <h3 ref={verificationHeading} tabIndex={-1} className="mt-3 text-base font-medium outline-none">
            {unavailable || blocked ? "Check your connection" : receiving ? "Traces received" : "Run one task"}
          </h3>
        )}
        <div
          className={cn(
            "inline-flex items-center gap-2 rounded-md px-2.5 py-1.5 text-xs font-medium",
            verifying ? "mt-4" : "lg:mt-4",
            receiving
              ? "bg-success/10 text-success"
              : unavailable || blocked
                ? "bg-warning/10 text-warning"
                : verifying
                  ? "bg-indigo-600 text-white dark:bg-indigo-500"
                  : "bg-muted text-muted-foreground",
          )}
          role="status"
          aria-live="polite"
        >
          <Activity aria-hidden="true" className="size-4 shrink-0 motion-safe:animate-pulse" />
          <p>{title}</p>
        </div>
        <p className="mt-2 text-xs leading-5 text-muted-foreground lg:text-sm lg:leading-6">{guidance}</p>
        {(receipt.match || !desktop || unavailable || (blocked && onSetup)) && (
          <div className="mt-3 flex flex-wrap items-center gap-2">
            {receipt.match && !authStatus && (
              <Button size="sm" onClick={() => onOpenTraces(agentName)}>
                View traces <ArrowRight className="size-3.5" aria-hidden="true" />
              </Button>
            )}
            {receiving && onConnectGitHub && (
              <Button size="sm" variant="outline" onClick={() => onConnectGitHub(agentName)}>
                <Github className="size-3.5" aria-hidden="true" /> Connect GitHub
              </Button>
            )}
            {authStatus === 401 && (
              <Button size="sm" onClick={() => window.location.reload()}>
                Reload to sign in
              </Button>
            )}
            {unavailable && !authStatus && (
              <Button
                size="sm"
                variant="outline"
                disabled={checking}
                onClick={() => {
                  void service.refetch();
                  receipt.refresh();
                }}
              >
                Retry connection
              </Button>
            )}
            {!desktop && (
              <Button
                size="sm"
                variant="ghost"
                className="px-0 text-xs text-muted-foreground"
                aria-expanded={detailsOpen}
                aria-controls={detailsId}
                onClick={() => setDetailsOpen((open) => !open)}
              >
                Connection details
                <ChevronDown aria-hidden="true" className={cn("size-3.5", detailsOpen && "rotate-180")} />
              </Button>
            )}
            {blocked && onSetup && (
              <Button variant="link" size="sm" className="px-0 text-xs" onClick={onSetup}>
                Deployment setup
              </Button>
            )}
          </div>
        )}
      </div>
      <div
        id={detailsId}
        hidden={!desktop && !detailsOpen}
        className={cn("space-y-5 border-t", verifying ? "p-5 lg:border-t-0 lg:border-l lg:bg-muted/10" : "py-4 lg:p-5")}
      >
        <dl className="space-y-3 text-sm">
          <ConnectionCheck
            label="Lens API"
            ready={service.error ? false : apiReady}
            success="Reachable"
            failure="Unavailable"
          />
          <ConnectionCheck label="Trace storage" ready={storageReady} success="Ready" failure="Not ready" />
          {keyReady !== undefined && keyReady !== null && (
            <ConnectionCheck
              label="Tracing key"
              ready={keyReady}
              success="Ready to use"
              failure="Activation unconfirmed"
            />
          )}
        </dl>
        <div>
          <p className="text-xs text-muted-foreground">Watching for agent</p>
          <p className="mt-1 font-mono text-xs break-all">{agentName || "Name your agent to begin"}</p>
          {receipt.match && (
            <dl className="mt-3 space-y-2 border-t pt-3 text-xs">
              <div className="flex justify-between gap-3">
                <dt className="text-muted-foreground">Runs in the last 14 days</dt>
                <dd className="font-medium tabular-nums">{receipt.match.runs.toLocaleString()}</dd>
              </div>
              <div className="flex flex-wrap justify-between gap-1">
                <dt className="text-muted-foreground">Last run started</dt>
                <dd className="font-medium">{new Date(receipt.match.last_seen).toLocaleString()}</dd>
              </div>
            </dl>
          )}
        </div>
        {verifying && !receiving && agentName && !unavailable && !blocked && (
          <p className="text-xs leading-5 text-muted-foreground">
            Nothing arriving? Check the endpoint and tracing key, restart your project, and confirm its trace name is{" "}
            <span className="font-medium text-foreground">{agentName}</span>.
          </p>
        )}
        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={checking || !enabled}
            onClick={() => {
              void service.refetch();
              receipt.refresh();
            }}
          >
            <RefreshCw aria-hidden="true" className={cn("size-3.5", checking && "motion-safe:animate-spin")} />
            {checking ? "Checking…" : "Check connection"}
          </Button>
        </div>
        {receipt.lastChecked !== null && (
          <p className="font-mono text-[11px] text-muted-foreground">
            Last checked {new Date(receipt.lastChecked).toLocaleTimeString()}
          </p>
        )}
      </div>
    </section>
  );
}

function ConnectionCheck({
  label,
  ready,
  success,
  failure,
}: {
  readonly label: string;
  readonly ready: boolean | null;
  readonly success: string;
  readonly failure: string;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <dt className="text-muted-foreground">{label}</dt>
      <dd
        className={cn(
          "flex items-center gap-1.5 text-xs font-medium",
          ready ? "text-success" : "text-muted-foreground",
        )}
      >
        {ready ? <Check aria-hidden="true" className="size-3.5" /> : <Circle aria-hidden="true" className="size-2" />}
        {ready === null ? "Checking" : ready ? success : failure}
      </dd>
    </div>
  );
}
