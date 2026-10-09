"use client";

import { useQuery } from "@tanstack/react-query";
import { lensQueries } from "../../data/queries";
import { useLensApi } from "../../data/LensServices";
import type { AnalysisModelInfo } from "../../model/types";

export interface AnalysisModels {
  readonly models: string[];
  readonly modelDetails: AnalysisModelInfo[];
  readonly modelsLoading: boolean;
  readonly modelsError?: string;
  readonly defaultModel?: string;
}

export function useAnalysisModels(): AnalysisModels {
  const api = useLensApi();
  const models = useQuery(lensQueries.models(api));
  const modelDetails = useQuery(lensQueries.modelDetails(api));
  const unsupported = new Set(
    modelDetails.data?.data.filter((model) => model.mode && model.mode !== "chat").map((model) => model.model_group),
  );
  const configured = models.data?.data.map((model) => model.id).filter((id) => !unsupported.has(id)) ?? [];
  return {
    models: configured,
    modelDetails: modelDetails.data?.data ?? [],
    modelsLoading: models.isLoading || modelDetails.isLoading,
    modelsError: models.error?.message ?? modelDetails.error?.message,
    defaultModel: configured.length === 1 ? configured[0] : undefined,
  };
}
