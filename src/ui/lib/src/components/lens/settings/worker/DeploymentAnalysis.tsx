"use client";

import type { ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "../../../ui/button";
import { useLensApi } from "../../data/LensServices";
import { lensQueries } from "../../data/queries";
import { useWorkerConnected } from "../../hooks/useWorkerConnected";
import type { LensList } from "../../model/types";
import { SettingsCard } from "../SettingsSection";

const CONFIGURATION_GUIDE = "https://github.com/BerriAI/lens/blob/main/docs/analysis.md";

export function DeploymentAnalysis({
  workers,
  readyAction,
}: {
  workers: LensList["workers"];
  readyAction?: ReactNode;
}) {
  const api = useLensApi();
  const models = useQuery(lensQueries.modelDetails(api));
  const list = useQuery(lensQueries.list(api));
  const connected = useWorkerConnected(list.data?.workers ?? workers);
  const configured = models.data?.data.filter((model) => model.mode !== "evaluation") ?? [];
  const refresh = () => {
    void models.refetch();
    void list.refetch();
  };
  return (
    <SettingsCard className="space-y-4">
      {models.isPending ? (
        <p role="status" className="text-sm text-muted-foreground">
          Checking analysis models…
        </p>
      ) : models.isError || list.isError ? (
        <p role="alert" className="text-sm text-destructive">
          Could not check analysis configuration. {models.error?.message ?? list.error?.message}
        </p>
      ) : configured.length === 0 ? (
        <div className="space-y-2">
          <h3 className="font-semibold">Add an analysis provider</h3>
          <p className="text-sm text-muted-foreground">
            Configure a model and its provider key in your Lens deployment, then restart Lens and check again. You can
            keep recording and inspecting traces while analysis is unconfigured.
          </p>
        </div>
      ) : (
        <div className="space-y-3">
          <p role="status" className="text-sm">
            {connected ? "Analysis is configured" : "Models are configured; waiting for the investigation worker"}
          </p>
          <ul aria-label="Analysis models" className="space-y-1 text-sm">
            {configured.map((model) => (
              <li key={model.model_group}>
                <span className="font-medium">{model.model_group}</span>
                <span className="text-muted-foreground"> · {model.providers.join(", ")}</span>
              </li>
            ))}
          </ul>
          <p className="text-sm text-muted-foreground">
            Investigations send selected trace content to these providers. Choose a model and monthly spending limit
            when creating an investigation. Provider keys stay on the Lens server.
          </p>
          {!connected && (
            <p className="text-sm text-muted-foreground">
              Check the Lens service logs if the worker does not become ready, then retry.
            </p>
          )}
        </div>
      )}
      <div className="flex flex-wrap items-center gap-3">
        <a
          href={CONFIGURATION_GUIDE}
          target="_blank"
          rel="noopener noreferrer"
          className="text-sm underline underline-offset-4"
        >
          Configure analysis models
        </a>
        <Button variant="outline" size="sm" onClick={refresh} disabled={models.isFetching || list.isFetching}>
          Check configuration
        </Button>
      </div>
      {connected && configured.length > 0 && !models.isError && !list.isError && readyAction}
    </SettingsCard>
  );
}
