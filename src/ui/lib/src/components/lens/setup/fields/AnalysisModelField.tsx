"use client";

import { Controller, useFormContext } from "react-hook-form";
import { SearchSelect } from "../../../shared/SearchSelect";
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
  return (
    <div className="space-y-2">
      <p className="text-sm font-medium">Analysis model</p>
      <Controller
        control={control}
        name="selectedModel"
        render={({ field }) => (
          <SearchSelect
            aria-label="Analysis model"
            options={analysisModelOptions(models.models, models.modelDetails)}
            value={model}
            onValueChange={(value) => field.onChange(value ?? "")}
            placeholder={models.modelsLoading ? "Loading models…" : "Choose a model"}
            disabled={models.modelsLoading}
            emptyText="No matching analysis models configured for Lens"
          />
        )}
      />
      {models.modelsError && (
        <p role="alert" className="text-sm text-destructive">
          Could not load models: {models.modelsError}
        </p>
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
