"use client";

import { Controller, useFormContext } from "react-hook-form";
import { useIsMutating } from "@tanstack/react-query";
import { SearchSelect } from "../../../shared/SearchSelect";
import { Button } from "../../../ui/button";
import { useOnboarding } from "../../onboarding/OnboardingContext";
import { useLensApi } from "../../data/LensServices";
import { lensKeys } from "../../data/queries";
import { analysisModelOptions, type ModelGate } from "./analysisModels";
import type { AnalysisModels } from "./useAnalysisModels";
import type { InvestigationInput } from "../investigationSchema";

export interface AnalysisModelFieldProps {
  readonly models: AnalysisModels;
  readonly gate: ModelGate;
  readonly model: string;
}

export function AnalysisModelField({ models, gate, model }: AnalysisModelFieldProps) {
  const { control } = useFormContext<InvestigationInput>();
  const { connect } = useOnboarding();
  const api = useLensApi();
  const connecting = useIsMutating({ mutationKey: lensKeys.gateway(api.scope) }) > 0;
  const options = analysisModelOptions(models.models, models.modelDetails);
  return (
    <div className="space-y-2">
      <p className="text-sm font-medium">Analysis model</p>
      <Controller
        control={control}
        name="selectedModel"
        render={({ field }) => (
          <SearchSelect
            aria-label="Analysis model"
            options={options}
            value={model}
            onValueChange={(value) => field.onChange(value ?? "")}
            placeholder={
              connecting ? "Loading gateway models…" : models.modelsLoading ? "Loading models…" : "Choose a model"
            }
            disabled={models.modelsLoading || connecting}
            emptyText={options.length ? "No models match your search" : "No analysis models available"}
          />
        )}
      />
      {models.modelsError && (
        <p role="alert" className="text-sm text-destructive">
          Could not load models: {models.modelsError}
        </p>
      )}
      {!connecting && !models.modelsLoading && !models.modelsError && options.length === 0 && (
        <div className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
          <p>No analysis models are available. Check the model gateway in Settings.</p>
          <Button type="button" variant="link" size="sm" onClick={connect}>
            Open Settings
          </Button>
        </div>
      )}
      {gate.unavailable && (
        <p role="alert" className="text-sm text-destructive">
          {model} is no longer available. Choose another analysis model.
        </p>
      )}
      {gate.unsupported && (
        <p role="alert" className="text-sm text-destructive">
          Choose a chat model that supports JSON output.
        </p>
      )}
    </div>
  );
}
