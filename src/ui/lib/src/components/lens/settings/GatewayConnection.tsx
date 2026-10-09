"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Network, RefreshCw } from "lucide-react";
import { StatusDot } from "../../shared/StatusDot";
import { Button } from "../../ui/button";
import { useLensApi } from "../data/LensServices";
import { lensKeys, lensQueries } from "../data/queries";
import { useOnboarding } from "../onboarding/OnboardingContext";
import { SettingsCard, SettingsSection } from "./SettingsSection";

export function GatewayConnection() {
  const api = useLensApi();
  const queryClient = useQueryClient();
  const { readOnly, canInvestigate } = useOnboarding();
  const gateway = useQuery(lensQueries.gateway(api));
  const refresh = useMutation({
    mutationFn: () => api.refreshGateway(),
    onSuccess: async (status) => {
      queryClient.setQueryData(lensKeys.gateway(api.scope), status);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: lensKeys.models(api.scope) }),
        queryClient.invalidateQueries({ queryKey: lensKeys.modelDetails(api.scope) }),
      ]);
    },
  });
  const status = gateway.data;
  const error = refresh.error?.message ?? gateway.error?.message ?? status?.error;
  const busy = gateway.isFetching || refresh.isPending;

  return (
    <SettingsSection
      heading="Model gateway"
      icon={<Network aria-hidden="true" className="size-4" />}
      description="Use models from your LiteLLM gateway for signals and investigations."
    >
      <SettingsCard className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p role="status" className="inline-flex items-center gap-2 text-sm font-medium">
            <StatusDot state={error ? "error" : status?.connected ? "ok" : "off"} />
            {gateway.isPending
              ? "Checking gateway…"
              : error
                ? "Gateway connection unavailable"
                : status?.connected
                  ? "Connected to LiteLLM"
                  : status?.configured
                    ? "Gateway is not connected"
                    : "No model gateway configured"}
          </p>
          {status?.configured && !readOnly && canInvestigate && (
            <Button variant="outline" size="sm" disabled={busy} onClick={() => refresh.mutate()}>
              <RefreshCw aria-hidden="true" className={`size-3.5 ${busy ? "animate-spin" : ""}`} />
              {refresh.isPending ? "Refreshing…" : "Refresh models"}
            </Button>
          )}
          {gateway.isError && !status && (
            <Button variant="outline" size="sm" disabled={busy} onClick={() => void gateway.refetch()}>
              Retry connection check
            </Button>
          )}
        </div>
        {status?.api_base && <p className="break-all font-mono text-xs text-muted-foreground">{status.api_base}</p>}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        {status?.configured ? (
          <>
            <p className="text-xs text-muted-foreground">
              {status.analysis_models} analysis models · {status.evaluation_models} evaluation models
              {!status.connected && status.last_refreshed && " · Last available catalogue"}
            </p>
            {status.connected && status.evaluation_models === 0 && (
              <p className="text-xs text-muted-foreground">
                No evaluation models found. Signals needs a model that supports the Decisions API.
              </p>
            )}
            {status.last_refreshed && (
              <p className="text-xs text-muted-foreground">
                Last refreshed{" "}
                <time dateTime={status.last_refreshed}>{new Date(status.last_refreshed).toLocaleString()}</time>
              </p>
            )}
            <p className="text-xs text-muted-foreground">Gateway credentials stay on the Lens server.</p>
          </>
        ) : !gateway.isPending && !error ? (
          <p className="text-xs text-muted-foreground">
            Add your gateway URL and key to the Lens deployment to discover its models.{" "}
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
