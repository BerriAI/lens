import type { AnalysisModelInfo } from "./types";

const modelNameCollator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });

export function compareModelNames(a: string, b: string): number {
  return modelNameCollator.compare(a, b) || (a < b ? -1 : a > b ? 1 : 0);
}

export function sortAnalysisModels(models: readonly AnalysisModelInfo[]): AnalysisModelInfo[] {
  return models.toSorted((a, b) => compareModelNames(a.model_group, b.model_group));
}

export function filterAnalysisModels(models: readonly AnalysisModelInfo[], search: string): AnalysisModelInfo[] {
  const query = search.trim().toLowerCase();
  return models.filter((model) => model.model_group.toLowerCase().includes(query));
}
