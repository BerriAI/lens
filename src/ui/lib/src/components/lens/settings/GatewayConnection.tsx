"use client";

import { useEffect, useRef } from "react";
import {
  useIsMutating,
  useMutation,
  useMutationState,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { Network, RefreshCw } from "lucide-react";
import { useLensHost } from "../../../host/LensHost";
import { hostedApiRoot } from "../../../lib/http/resolveApiBase";
import { StatusDot } from "../../shared/StatusDot";
import { Button } from "../../ui/button";
import { useLensApi } from "../data/LensServices";
import { lensKeys, lensQueries } from "../data/queries";
import { useOnboarding } from "../onboarding/OnboardingContext";
import { SettingsCard, SettingsSection } from "./SettingsSection";

export function GatewayConnection({ automatic = false }: { automatic?: boolean }) {
  const api = useLensApi();
  const queryClient = useQueryClient();
  const { readOnly, canInvestigate } = useOnboarding();
  const host = useLensHost();
  const embedded =
    host.surface === "embedded" ||
    (typeof window !== "undefined" &&
      hostedApiRoot(window.location.pathname) !== null);
  const enabled = !automatic || (embedded && !readOnly && canInvestigate);
  const attempted = useRef(false);
  const gateway = useQuery({ ...lensQueries.gateway(api), enabled });
  const pending = useIsMutating({ mutationKey: lensKeys.gateway(api.scope) });
  const refresh = useMutation({
    mutationKey: lensKeys.gateway(api.scope),
    retry: false,
    mutationFn: (connect: boolean) =>
      connect ? api.connectGateway() : api.refreshGateway(),
    onSuccess: async (status) => {
      queryClient.setQueryData(lensKeys.gateway(api.scope), status);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: lensKeys.models(api.scope) }),
        queryClient.invalidateQueries({
          queryKey: lensKeys.modelDetails(api.scope),
        }),
      ]);
    },
  });
  const mutations = useMutationState({
    filters: { mutationKey: lensKeys.gateway(api.scope), exact: true },
    select: (mutation) => ({
      pending: mutation.state.status === "pending",
      connecting: mutation.state.variables === true,
      error: mutation.state.error,
    }),
  });
  const latest = mutations.at(-1);
  const status = gateway.data;
  const error =
    latest?.error?.message ??
    gateway.error?.message ??
    (status?.connected ? null : status?.error);
  const warning = status?.connected ? status.error : null;
  const busy = gateway.isFetching || pending > 0;
  const canConnect =
    embedded &&
    status?.supports_gateway_connect &&
    !status.configured &&
    !readOnly &&
    canInvestigate;
  const { mutate } = refresh;

  useEffect(() => {
    if (!automatic || !canConnect || attempted.current || busy) return;
    attempted.current = true;
    mutate(true);
  }, [automatic, canConnect, busy, mutate]);

  if (automatic) return null;

  return (
    <SettingsSection
      heading="Model gateway"
      icon={<Network aria-hidden="true" className="size-4" />}
      description="Use models from your LiteLLM gateway for signals and findings."
    >
      <SettingsCard className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p
            role="status"
            className="inline-flex items-center gap-2 text-sm font-medium"
          >
            <StatusDot
              state={error ? "error" : status?.connected ? "ok" : "off"}
            />
            {latest?.pending && latest.connecting
              ? "Connecting models from this gateway…"
              : gateway.isPending
                ? "Checking gateway…"
                : error
                  ? "Gateway connection unavailable"
                  : status?.connected
                    ? "Connected to LiteLLM"
                    : status?.configured
                      ? "Gateway is not connected"
                      : canConnect
                        ? "Connect models from this gateway"
                        : status?.login_connected
                          ? "Connected for login, add a key for models"
                          : "No model gateway configured"}
          </p>
          {status?.configured && !readOnly && canInvestigate && (
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              onClick={() => refresh.mutate(false)}
            >
              <RefreshCw
                aria-hidden="true"
                className={`size-3.5 ${busy ? "animate-spin" : ""}`}
              />
              {latest?.pending ? "Refreshing…" : "Refresh models"}
            </Button>
          )}
          {canConnect && (
            <Button
              size="sm"
              disabled={busy}
              onClick={() => refresh.mutate(true)}
            >
              {latest?.pending ? "Connecting…" : "Use this LiteLLM gateway"}
            </Button>
          )}
          {gateway.isError && !status && (
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              onClick={() => void gateway.refetch()}
            >
              Retry connection check
            </Button>
          )}
        </div>
        {status?.api_base && (
          <p className="break-all font-mono text-xs text-muted-foreground">
            {status.api_base}
          </p>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        {!error && warning && (
          <p
            role="status"
            className="text-xs text-amber-700 dark:text-amber-300"
          >
            {warning}
          </p>
        )}
        {status?.configured ? (
          <>
            <p className="text-xs text-muted-foreground">
              {status.analysis_models} analysis models ·{" "}
              {status.evaluation_models} evaluation models
              {!status.connected &&
                status.last_refreshed &&
                " · Last available catalogue"}
            </p>
            {status.connected && status.evaluation_models === 0 && (
              <p className="text-xs text-muted-foreground">
                No evaluation models found. Signals needs a model that supports
                the Decisions API.
              </p>
            )}
            {status.last_refreshed && (
              <p className="text-xs text-muted-foreground">
                Last refreshed{" "}
                <time dateTime={status.last_refreshed}>
                  {new Date(status.last_refreshed).toLocaleString()}
                </time>
              </p>
            )}
            <p className="text-xs text-muted-foreground">
              Manage model access, budgets, and rate limits in LiteLLM Virtual
              Keys. Refresh here after adding models. Gateway credentials stay
              on the Lens server.
            </p>
          </>
        ) : canConnect ? (
          <p className="text-xs text-muted-foreground">
            Connect the gateway you are using now. LiteLLM creates a key for all
            gateway models and sends it directly to Lens. You can manage the key
            in LiteLLM Virtual Keys.
          </p>
        ) : !gateway.isPending && !error ? (
          <p className="text-xs text-muted-foreground">
            Add your gateway URL and key to the Lens deployment to discover its
            models.{" "}
            <a
              href="https://github.com/BerriAI/lens/blob/main/docs/analysis.md"
              target="_blank"
              rel="noopener noreferrer"
              className="text-foreground underline underline-offset-4"
            >
              Configure gateway
            </a>
          </p>
        ) : null}
      </SettingsCard>
    </SettingsSection>
  );
}
